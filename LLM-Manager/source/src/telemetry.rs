use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use ureq::{Agent, Body, http::Response};

const DEFAULT_OLLAMA_ADDRESS: &str = "127.0.0.1:11434";
const ADDRESS_OVERRIDE: &str = "LLM_MANAGER_OLLAMA_ADDRESS";
const READ_TIMEOUT: Duration = Duration::from_secs(3);
const MODEL_TIMEOUT: Duration = Duration::from_secs(90);
const GPU_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const GPU_FIELDS: &str =
    "--query-gpu=index,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw";

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub engine: Result<Engine, String>,
    pub gpus: Result<Vec<Gpu>, String>,
}

#[derive(Clone, Debug, Default)]
pub struct Engine {
    pub version: Option<String>,
    pub models: Vec<Model>,
    pub residents: Vec<Resident>,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub size: u64,
    pub details: ModelDetails,
    pub max_context: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ModelDetails {
    pub family: String,
    pub parameter_size: String,
    pub quantization_level: String,
}

#[derive(Clone, Debug, Default)]
pub struct Resident {
    pub name: String,
    pub size: u64,
    pub size_vram: u64,
    pub context_length: Option<u64>,
    pub expires_at: Option<String>,
    pub runner: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub index: u32,
    pub name: String,
    pub utilization: Option<f32>,
    pub memory_used_mib: Option<f32>,
    pub memory_total_mib: Option<f32>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
}

#[derive(Default, Deserialize)]
struct WireDetails {
    #[serde(flatten)]
    display: ModelDetails,
    context_length: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct WireModel {
    name: String,
    model: String,
    size: u64,
    size_vram: u64,
    context_length: Option<u64>,
    expires_at: Option<String>,
    runner: Option<String>,
    remote_host: Option<String>,
    remote_model: Option<String>,
    details: WireDetails,
}

impl WireModel {
    fn name(&self) -> &str {
        if self.name.is_empty() {
            &self.model
        } else {
            &self.name
        }
    }

    fn is_local(&self) -> bool {
        let remote_fields = [self.remote_host.as_deref(), self.remote_model.as_deref()];
        remote_fields.into_iter().flatten().all(str::is_empty)
            && is_local_name(self.name())
            && (self.model.is_empty() || is_local_name(&self.model))
    }

    fn is_installed_local(&self) -> bool {
        self.size > 0 && self.is_local()
    }

    fn installed(self) -> Model {
        Model {
            name: self.name().to_owned(),
            size: self.size,
            max_context: self
                .details
                .context_length
                .or(self.context_length)
                .filter(|size| *size > 0),
            details: self.details.display,
        }
    }

    fn resident(self) -> Resident {
        Resident {
            name: self.name().to_owned(),
            size: self.size,
            size_vram: self.size_vram,
            context_length: self.context_length,
            expires_at: self.expires_at,
            runner: self.runner,
        }
    }
}

#[derive(Deserialize)]
struct ModelList {
    models: Vec<WireModel>,
}

#[derive(Deserialize)]
struct Version {
    version: String,
}

struct Ollama<'a> {
    address: &'a str,
    agent: Agent,
}

impl<'a> Ollama<'a> {
    fn new(address: &'a str) -> Self {
        let config = Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_connect(Some(READ_TIMEOUT))
            .timeout_global(Some(READ_TIMEOUT))
            .build();
        Self {
            address,
            agent: config.into(),
        }
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let url = format!("http://{}{path}", self.address);
        let response = self
            .agent
            .get(url.as_str())
            .call()
            .map_err(|error| format!("Ollama {path} on {}: {error}", self.address))?;
        decode(response, path)
    }

    fn engine(&self) -> Result<Engine, String> {
        let models: ModelList = self.get("/api/tags")?;
        let residents: ModelList = self.get("/api/ps")?;
        let version = self
            .get::<Version>("/api/version")
            .ok()
            .map(|version| version.version)
            .filter(|version| !version.is_empty());
        Ok(Engine {
            version,
            models: models
                .models
                .into_iter()
                .filter(WireModel::is_installed_local)
                .map(WireModel::installed)
                .collect(),
            residents: residents
                .models
                .into_iter()
                .filter(WireModel::is_local)
                .map(WireModel::resident)
                .collect(),
        })
    }

    fn set_resident(&self, name: &str, load: bool) -> Result<(), String> {
        if !is_local_name(name) {
            return Err(String::from("Choose an installed local model."));
        }
        let inventory: ModelList = self.get("/api/tags")?;
        if !inventory
            .models
            .iter()
            .any(|model| model.name() == name && model.is_installed_local())
        {
            return Err(format!("{name} is no longer installed locally."));
        }

        let action = if load { "load" } else { "unload" };
        let keep_alive = if load { json!("5m") } else { json!(0) };
        // Ollama's explicit source suffix rejects remote aliases on the server as well.
        let payload = json!({
            "model": format!("{name}:local"),
            "stream": false,
            "keep_alive": keep_alive,
        });
        let path = "/api/generate";
        let url = format!("http://{}{path}", self.address);
        let response = self
            .agent
            .post(url.as_str())
            .config()
            .timeout_global(Some(MODEL_TIMEOUT))
            .build()
            .header("Content-Type", "application/json")
            .send(payload.to_string().as_bytes())
            .map_err(|error| format!("Ollama could not {action} {name}: {error}"))?;
        let result: Value = decode(response, path)?;
        if result.get("done").and_then(Value::as_bool) != Some(true)
            || result.get("done_reason").and_then(Value::as_str) != Some(action)
        {
            return Err(format!(
                "Ollama did not acknowledge the {action} request for {name}."
            ));
        }
        Ok(())
    }
}

fn is_local_name(name: &str) -> bool {
    if name.is_empty() || name.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
        return false;
    }
    let tag = name
        .rsplit_once(':')
        .map(|(_, tag)| tag.to_ascii_lowercase());
    !tag.is_some_and(|tag| tag == "cloud" || tag == "local" || tag.ends_with("-cloud"))
}

fn decode<T: DeserializeOwned>(mut response: Response<Body>, path: &str) -> Result<T, String> {
    let status = response.status();
    if status.is_redirection() {
        return Err(format!(
            "Ollama {path} returned an unexpected redirect ({status})."
        ));
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|error| format!("Could not read Ollama {path}: {error}"))?;
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|body| body.get("error").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default();
        let clean_message: String = message
            .chars()
            .filter(|ch| !ch.is_control())
            .take(350)
            .collect();
        return Err(if clean_message.is_empty() {
            format!("Ollama {path} returned HTTP {status}.")
        } else {
            format!("Ollama {path}: {clean_message}")
        });
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid Ollama {path} response: {error}"))
}

fn number(field: &str) -> Option<f32> {
    field
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_gpu_output(output: &str) -> Result<Vec<Gpu>, String> {
    let mut gpus = Vec::new();
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let fields: Vec<_> = line.rsplitn(6, ',').collect();
        let [power, temperature, total, used, utilization, head] = fields.as_slice() else {
            return Err(String::from("nvidia-smi returned an unexpected GPU row."));
        };
        let (index, name) = head
            .split_once(',')
            .ok_or_else(|| String::from("nvidia-smi returned an unexpected GPU name."))?;
        let index = index
            .trim()
            .parse()
            .map_err(|_| String::from("nvidia-smi returned an invalid GPU index."))?;
        gpus.push(Gpu {
            index,
            name: name.trim().trim_matches('"').to_owned(),
            utilization: number(utilization),
            memory_used_mib: number(used),
            memory_total_mib: number(total),
            temperature_c: number(temperature),
            power_w: number(power),
        });
    }
    if gpus.is_empty() {
        return Err(String::from("nvidia-smi did not report a GPU."));
    }
    Ok(gpus)
}

fn read_gpus() -> Result<Vec<Gpu>, String> {
    let mut child = Command::new("nvidia-smi")
        .args([GPU_FIELDS, "--format=csv,noheader,nounits"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start nvidia-smi: {error}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < GPU_TIMEOUT => thread::sleep(Duration::from_millis(25)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(error) => format!("Could not query nvidia-smi: {error}"),
                    _ => String::from("nvidia-smi did not respond in time."),
                });
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Could not read nvidia-smi: {error}"))?;
    if !output.status.success() {
        return Err(format!("nvidia-smi failed ({})", output.status));
    }
    parse_gpu_output(&String::from_utf8_lossy(&output.stdout))
}

/// The Ollama address. `LLM_MANAGER_OLLAMA_ADDRESS` may point at another loopback
/// port (for a test stub); anything that is not loopback is ignored.
pub fn ollama_address() -> &'static str {
    static ADDRESS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ADDRESS.get_or_init(|| {
        std::env::var(ADDRESS_OVERRIDE)
            .ok()
            .filter(|address| is_loopback_address(address))
            .unwrap_or_else(|| DEFAULT_OLLAMA_ADDRESS.to_owned())
    })
}

fn is_loopback_address(address: &str) -> bool {
    address
        .parse::<std::net::SocketAddr>()
        .is_ok_and(|socket| socket.ip().is_loopback())
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        engine: Ollama::new(ollama_address()).engine(),
        gpus: read_gpus(),
    }
}

pub fn set_resident(name: &str, load: bool) -> Result<(), String> {
    Ollama::new(ollama_address()).set_resident(name, load)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    struct Step {
        request: &'static str,
        body: Option<Value>,
        status: &'static str,
        reply: Value,
    }

    fn get_step(path: &'static str, reply: Value) -> Step {
        Step {
            request: path,
            body: None,
            status: "200 OK",
            reply,
        }
    }

    fn stub(steps: Vec<Step>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let worker = thread::spawn(move || {
            for step in steps {
                let (mut conn, _) = listener.accept().unwrap();
                conn.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                {
                    let mut request = BufReader::new(&mut conn);
                    let mut line = String::new();
                    assert_ne!(request.read_line(&mut line).unwrap(), 0);
                    assert_eq!(line.trim_end(), step.request);
                    let mut body_length = 0;
                    loop {
                        line.clear();
                        assert_ne!(request.read_line(&mut line).unwrap(), 0);
                        if line.trim_end().is_empty() {
                            break;
                        }
                        if let Some((key, value)) = line.split_once(':') {
                            if key.eq_ignore_ascii_case("content-length") {
                                body_length = value.trim().parse().unwrap();
                            }
                        }
                    }
                    if let Some(expected) = step.body {
                        let mut body = vec![0; body_length];
                        request.read_exact(&mut body).unwrap();
                        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), expected);
                    } else {
                        assert_eq!(body_length, 0);
                    }
                }
                let payload = step.reply.to_string();
                write!(
                    conn,
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    step.status,
                    payload.len(),
                    payload
                )
                .unwrap();
            }
        });
        (address, worker)
    }

    fn inventory() -> Value {
        json!({
            "models": [
                {"name":"local:4b", "size":100, "details":{
                    "family":"example", "parameter_size":"4B", "quantization_level":"Q4", "context_length":8192
                }},
                {"model":"second:latest","size":20},
                {"name":"bad:cloud","size":100},
                {"name":"bad:4b-cloud","size":100},
                {"name":"remote-alias:latest","size":100,"remote_host":"https://example.test","remote_model":"target"},
                {"name":"zero:latest","size":0}
            ]
        })
    }

    #[test]
    fn snapshot_separates_disk_inventory_and_loaded_runners() {
        let (address, worker) = stub(vec![
            get_step("GET /api/tags HTTP/1.1", inventory()),
            get_step(
                "GET /api/ps HTTP/1.1",
                json!({"models":[{"name":"local:4b", "size":125, "size_vram":80,
                    "context_length":4096, "runner":"llamacpp"}]}),
            ),
            get_step("GET /api/version HTTP/1.1", json!({"version":"0.40.14"})),
        ]);
        let engine = Ollama::new(&address).engine().unwrap();
        worker.join().unwrap();
        assert_eq!(engine.version.as_deref(), Some("0.40.14"));
        assert_eq!(engine.models.len(), 2);
        assert_eq!(engine.models[0].size, 100);
        assert_eq!(engine.models[0].details.quantization_level, "Q4");
        assert_eq!(engine.models[0].max_context, Some(8192));
        assert_eq!(engine.models[1].name, "second:latest");
        assert_eq!(engine.residents[0].size_vram, 80);
        assert_eq!(engine.residents[0].context_length, Some(4096));
    }

    #[test]
    fn load_and_unload_use_only_empty_local_requests() {
        let (address, worker) = stub(vec![
            get_step("GET /api/tags HTTP/1.1", inventory()),
            Step {
                request: "POST /api/generate HTTP/1.1",
                body: Some(json!({"model":"local:4b:local","stream":false,"keep_alive":"5m"})),
                status: "200 OK",
                reply: json!({"done":true,"done_reason":"load"}),
            },
            get_step("GET /api/tags HTTP/1.1", inventory()),
            Step {
                request: "POST /api/generate HTTP/1.1",
                body: Some(json!({"model":"local:4b:local","stream":false,"keep_alive":0})),
                status: "200 OK",
                reply: json!({"done":true,"done_reason":"unload"}),
            },
            get_step("GET /api/tags HTTP/1.1", inventory()),
        ]);
        let ollama = Ollama::new(&address);
        ollama.set_resident("local:4b", true).unwrap();
        ollama.set_resident("local:4b", false).unwrap();
        let remote_error = ollama
            .set_resident("remote-alias:latest", true)
            .unwrap_err();
        assert!(remote_error.contains("no longer installed locally"));
        assert!(ollama.set_resident("bad:4b-cloud", true).is_err());
        worker.join().unwrap();
    }

    #[test]
    fn unavailable_residency_does_not_report_an_empty_snapshot() {
        let (address, worker) = stub(vec![
            get_step("GET /api/tags HTTP/1.1", inventory()),
            Step {
                request: "GET /api/ps HTTP/1.1",
                body: None,
                status: "503 Service Unavailable",
                reply: json!({"error":"runners temporarily unavailable"}),
            },
        ]);
        let error = Ollama::new(&address).engine().unwrap_err();
        worker.join().unwrap();
        assert!(error.contains("runners temporarily unavailable"));
    }

    #[test]
    fn address_override_accepts_only_loopback() {
        assert!(is_loopback_address("127.0.0.1:18080"));
        assert!(is_loopback_address("[::1]:11434"));
        assert!(!is_loopback_address("192.0.2.10:11434"));
        assert!(!is_loopback_address("example.test:11434"));
    }

    #[test]
    fn gpu_fields_allow_unavailable_values_and_commas_in_the_name() {
        let gpus = parse_gpu_output("0, \"RTX, test\", 19, 1400, 8192, 50, [N/A]\n").unwrap();
        assert_eq!(gpus[0].name, "RTX, test");
        assert_eq!(gpus[0].memory_used_mib, Some(1400.0));
        assert_eq!(gpus[0].power_w, None);
    }
}
