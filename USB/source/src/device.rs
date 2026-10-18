use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Partition {
    pub number: u32,
    pub size: u64,
    pub offset: u64,
    pub paths: Vec<Option<String>>,
    pub letter: String,
    pub path: String,
    pub mountpoints: Vec<Option<String>>,
    #[serde(rename = "type")]
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Device {
    pub path: String,
    pub number: u32,
    pub model: String,
    pub serial: String,
    pub unique_id: String,
    pub bus: String,
    pub size: u64,
    pub sector: u64,
    pub system: bool,
    pub read_only: bool,
    pub offline: bool,
    pub partition_style: String,
    pub partitions: Vec<Partition>,
    pub major_minor: String,
    pub mountpoints: Vec<String>,
}
impl Device {
    pub fn blocked(&self) -> Option<&'static str> {
        if !self.bus.eq_ignore_ascii_case("usb") {
            Some("Only USB-connected disks are supported")
        } else if self.system {
            Some("Contains a system, boot, swap or protected mount")
        } else if self.offline || self.read_only {
            Some("Disk is offline or read-only")
        } else if self.serial.is_empty() || self.unique_id.is_empty() {
            Some("Stable serial and device identity are required")
        } else if self.size < 1024 * 1024 || ![512, 4096].contains(&self.sector) {
            Some("No usable media or unsupported sector size")
        } else {
            None
        }
    }
    pub fn same_identity(&self, other: &Self) -> bool {
        self.path == other.path
            && self.serial == other.serial
            && self.unique_id == other.unique_id
            && self.model == other.model
            && self.size == other.size
            && self.bus == other.bus
            && self.major_minor == other.major_minor
            && self.sector == other.sector
    }
    pub fn token(&self) -> String {
        format!(
            "ERASE {}",
            self.serial
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        )
    }
}

pub fn command(mut cmd: Command) -> Result<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = cmd
        .output()
        .context("Could not start required system utility")?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        })
        .trim()
    );
    Ok(String::from_utf8(output.stdout)?
        .trim()
        .trim_start_matches('\u{feff}')
        .to_owned())
}
#[cfg(unix)]
fn cmd(program: &str, args: &[&str]) -> Result<String> {
    let mut c = Command::new(program);
    c.args(args);
    command(c)
}

#[cfg(windows)]
fn ps(script: &str) -> Result<String> {
    use base64::Engine;
    let wrapped = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new(); try {{ {script} }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}"
    );
    let bytes: Vec<_> = wrapped.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let path = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()))
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut c = Command::new(path);
    c.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded]);
    command(c)
}
#[cfg(windows)]
fn ps_data<T: Serialize>(data: &T) -> Result<String> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(data)?);
    Ok(format!(
        "([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}')) | ConvertFrom-Json)"
    ))
}

pub fn list() -> Result<Vec<Device>> {
    #[cfg(windows)]
    {
        Ok(serde_json::from_str(&ps(r#"
        $items=@(Get-Disk | ForEach-Object {
          $d=$_; $parts=@(Get-Partition -DiskNumber $d.Number -ErrorAction SilentlyContinue | ForEach-Object {
            [pscustomobject]@{number=$_.PartitionNumber;size=$_.Size;offset=$_.Offset;paths=@($_.AccessPaths);letter=[string]$_.DriveLetter;type=[string]$_.Type}
          });
          [pscustomobject]@{path=('\\.\PhysicalDrive'+$d.Number);number=$d.Number;model=$d.FriendlyName;serial=([string]$d.SerialNumber).Trim();unique_id=([string]$d.UniqueId).Trim();bus=[string]$d.BusType;size=[long]$d.Size;sector=[int]$d.LogicalSectorSize;system=([bool]$d.IsBoot -or [bool]$d.IsSystem -or [bool]$d.IsClustered);read_only=[bool]$d.IsReadOnly;offline=[bool]$d.IsOffline;partition_style=[string]$d.PartitionStyle;partitions=$parts}
        });ConvertTo-Json -InputObject $items -Depth 8 -Compress
        "#)?)?)
    }
    #[cfg(unix)]
    {
        parse_lsblk(&serde_json::from_str(&cmd(
            "lsblk",
            &[
                "--json",
                "--bytes",
                "--paths",
                "--output",
                "NAME,PATH,TYPE,SIZE,MODEL,SERIAL,WWN,TRAN,RO,MOUNTPOINTS,MAJ:MIN,LOG-SEC,PTTYPE",
            ],
        )?)?)
    }
}

pub fn parse_lsblk(doc: &Value) -> Result<Vec<Device>> {
    fn text(v: &Value, k: &str) -> String {
        v[k].as_str().unwrap_or("").trim().into()
    }
    fn descendants<'a>(v: &'a Value, result: &mut Vec<&'a Value>) {
        result.push(v);
        if let Some(children) = v["children"].as_array() {
            for c in children {
                descendants(c, result);
            }
        }
    }
    let mut result = vec![];
    for v in doc["blockdevices"]
        .as_array()
        .context("Missing block device list")?
    {
        if v["type"] != "disk" {
            continue;
        }
        let mut nodes = vec![];
        descendants(v, &mut nodes);
        let mounts: Vec<String> = nodes
            .iter()
            .flat_map(|n| {
                n["mountpoints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        let protected = [
            "/",
            "/boot",
            "/boot/efi",
            "/home",
            "/usr",
            "/var",
            "/opt",
            "/cdrom",
            "/isodevice",
            "/run/live/medium",
            "[SWAP]",
        ];
        let system = mounts
            .iter()
            .any(|m| protected.contains(&m.as_str()) || m.starts_with("/boot/"))
            || nodes.iter().skip(1).any(|n| {
                ["crypt", "lvm", "raid0", "raid1", "raid5", "raid6", "raid10"]
                    .contains(&n["type"].as_str().unwrap_or(""))
            });
        let serial = text(v, "serial");
        let wwn = text(v, "wwn");
        let partitions = nodes
            .iter()
            .skip(1)
            .map(|n| Partition {
                path: text(n, "path"),
                size: n["size"].as_u64().unwrap_or(0),
                mountpoints: n["mountpoints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|m| m.as_str().map(str::to_owned))
                    .collect(),
                kind: text(n, "type"),
                ..Default::default()
            })
            .collect();
        result.push(Device {
            path: text(v, "path"),
            model: text(v, "model"),
            unique_id: if wwn.is_empty() { serial.clone() } else { wwn },
            serial,
            bus: text(v, "tran"),
            size: v["size"].as_u64().context("Missing disk capacity")?,
            sector: v["log-sec"].as_u64().unwrap_or(512),
            system,
            read_only: v["ro"].as_bool().unwrap_or(true),
            partitions,
            mountpoints: mounts,
            major_minor: text(v, "maj:min"),
            partition_style: text(v, "pttype"),
            ..Default::default()
        });
    }
    Ok(result)
}

pub fn validate_snapshot(expected: &Device, current: &Device, required: u64) -> Result<()> {
    ensure!(
        expected.same_identity(current),
        "USB identity changed. Refresh and select it again."
    );
    if let Some(reason) = current.blocked() {
        bail!(reason);
    }
    ensure!(
        current.size >= required,
        "The USB is too small for the requested layout"
    );
    Ok(())
}
pub fn revalidate(expected: &Device, required: u64) -> Result<Device> {
    let found = list()?
        .into_iter()
        .find(|d| d.path == expected.path)
        .context("USB removed or no longer available")?;
    validate_snapshot(expected, &found, required)?;
    Ok(found)
}
pub fn assert_source_elsewhere(d: &Device, paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        #[cfg(windows)]
        {
            let s = path
                .canonicalize()?
                .to_string_lossy()
                .trim_start_matches("\\\\?\\")
                .to_string();
            let code = format!(
                "$p={};$v=Get-Volume -FilePath $p;$n=@($v|Get-Partition|Get-Disk|Select-Object -ExpandProperty Number);ConvertTo-Json -InputObject $n",
                ps_data(&s)?
            );
            let numbers: Vec<u32> = serde_json::from_str(&ps(&code)?)?;
            ensure!(!numbers.is_empty(), "Could not determine source disk");
            ensure!(
                !numbers.contains(&d.number),
                "Move the image and Axentra off the selected USB first"
            );
        }
        #[cfg(unix)]
        {
            let canonical = path.canonicalize()?;
            let p = canonical.to_str().context("Invalid path encoding")?;
            let source = cmd("findmnt", &["-n", "-o", "SOURCE", "--target", p])?;
            let source = source.split('[').next().unwrap_or("");
            ensure!(
                source.starts_with("/dev/"),
                "Source storage is not a recognized local block device"
            );
            let parents = cmd("lsblk", &["-s", "-n", "-p", "-o", "PATH", source])?;
            ensure!(
                !parents.lines().any(|p| p.trim() == d.path),
                "Move the image and Axentra off the target USB first"
            );
        }
    }
    Ok(())
}
pub fn admin() -> bool {
    #[cfg(windows)]
    {
        unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
    }
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
}

/// Serializes Axentra workers targeting the same Windows USB across app instances.
pub struct TransactionLock {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}
impl TransactionLock {
    pub fn acquire(device: &Device) -> Result<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError},
                System::Threading::CreateMutexW,
            };
            let name = format!(
                "Global\\AxentraUSB-{}",
                &crate::image::hash_bytes(device.unique_id.as_bytes())[..32]
            );
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let handle = unsafe { CreateMutexW(std::ptr::null(), 1, wide.as_ptr()) };
            ensure!(
                !handle.is_null(),
                "Could not acquire the device operation lock"
            );
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe {
                    CloseHandle(handle);
                }
                bail!("Another Axentra operation is already using this USB");
            }
            Ok(Self { handle })
        }
        #[cfg(unix)]
        {
            let _ = device;
            Ok(Self {})
        }
    }
}
impl Drop for TransactionLock {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
#[cfg(unix)]
fn unmount(d: &Device) -> Result<()> {
    for p in d.partitions.iter().rev() {
        if p.mountpoints.iter().any(Option::is_some) {
            cmd("umount", &["--", &p.path])?;
        }
    }
    if d.partitions.is_empty() && !d.mountpoints.is_empty() {
        cmd("umount", &["--", &d.path])?;
    }
    Ok(())
}

pub struct RawDevice {
    pub file: File,
    _locks: Vec<File>,
}
#[cfg(windows)]
fn ioctl(f: &File, code: u32, input: &[u8], output_len: usize) -> Result<Vec<u8>> {
    use std::os::windows::io::AsRawHandle;
    let mut result = vec![0u8; output_len];
    let mut returned = 0;
    let ok = unsafe {
        windows_sys::Win32::System::IO::DeviceIoControl(
            f.as_raw_handle(),
            code,
            if input.is_empty() {
                std::ptr::null()
            } else {
                input.as_ptr().cast()
            },
            input.len() as u32,
            if output_len == 0 {
                std::ptr::null_mut()
            } else {
                result.as_mut_ptr().cast()
            },
            output_len as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    ensure!(
        ok != 0,
        "Device control failed: {}",
        std::io::Error::last_os_error()
    );
    result.truncate(returned as usize);
    Ok(result)
}
pub fn open_raw(expected: &Device, write: bool) -> Result<RawDevice> {
    let d = revalidate(expected, 0)?;
    #[cfg(windows)]
    {
        let mut locks = vec![];
        use std::os::windows::fs::OpenOptionsExt;
        if write {
            for part in &d.partitions {
                for path in part
                    .paths
                    .iter()
                    .flatten()
                    .filter(|s| s.starts_with("\\\\?\\Volume{"))
                {
                    let f = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .share_mode(3)
                        .open(path.trim_end_matches('\\'))?;
                    ioctl(&f, 0x90018, &[], 0)
                        .context("USB is in use; close applications using it")?;
                    ioctl(&f, 0x90020, &[], 0)?;
                    locks.push(f);
                }
            }
        }
        revalidate(&d, 0)?;
        let file = OpenOptions::new()
            .read(true)
            .write(write)
            .share_mode(3)
            .custom_flags(if write { 0x80000000 } else { 0 })
            .open(&d.path)?;
        let number = ioctl(&file, 0x2D1080, &[], 12)?;
        let length = ioctl(&file, 0x7405C, &[], 8)?;
        let desc = ioctl(&file, 0x2D1400, &[0; 12], 4096)?;
        ensure!(
            number.len() >= 12 && length.len() >= 8 && desc.len() >= 28,
            "Incomplete device identity response"
        );
        let serial_offset = u32::from_le_bytes(desc[24..28].try_into()?) as usize;
        ensure!(
            serial_offset > 0 && serial_offset < desc.len(),
            "No serial from open device handle"
        );
        let serial = std::str::from_utf8(
            desc[serial_offset..]
                .split(|b| *b == 0)
                .next()
                .unwrap_or(&[]),
        )?
        .trim();
        ensure!(
            u32::from_le_bytes(number[4..8].try_into()?) == d.number
                && u64::from_le_bytes(length[..8].try_into()?) == d.size
                && serial == d.serial,
            "Open handle does not match the selected USB"
        );
        revalidate(&d, 0)?;
        Ok(RawDevice {
            file,
            _locks: locks,
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
            io::AsRawFd,
        };
        if write {
            unmount(&d)?;
            revalidate(&d, 0)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(write)
            .custom_flags(if write { libc::O_EXCL } else { 0 })
            .open(&d.path)?;
        let meta = file.metadata()?;
        ensure!(
            meta.file_type().is_block_device(),
            "Target is not a block device"
        );
        ensure!(
            format!("{}:{}", libc::major(meta.rdev()), libc::minor(meta.rdev())) == d.major_minor,
            "Block device handle changed"
        );
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "Device is busy"
        );
        revalidate(&d, 0)?;
        Ok(RawDevice {
            file,
            _locks: vec![],
        })
    }
}

pub struct Volumes {
    pub boot: PathBuf,
    pub data: PathBuf,
    #[cfg_attr(windows, allow(dead_code))]
    mounted: Vec<PathBuf>,
    _lock: Option<File>,
}
impl Volumes {
    #[allow(unused_mut)]
    pub fn finish(mut self) -> Result<()> {
        flush(&self.boot)?;
        flush(&self.data)?;
        #[cfg(unix)]
        {
            while let Some(p) = self.mounted.last() {
                cmd("umount", &["--", p.to_str().context("Invalid mount path")?])?;
                self.mounted.pop();
            }
        }
        Ok(())
    }
}
impl Drop for Volumes {
    fn drop(&mut self) {
        #[cfg(unix)]
        for p in self.mounted.iter().rev() {
            let _ = cmd("umount", &["--", &p.to_string_lossy()]);
        }
    }
}
pub fn create_volumes(expected: &Device, scratch: &Path) -> Result<Volumes> {
    let d = revalidate(expected, 0)?;
    #[cfg(windows)]
    {
        let _ = scratch;
        let script = format!("$e={};", ps_data(&d)?)
            + r#"
        function Assert-Target {
          $d=Get-Disk -Number $e.number;
          if ($d.BusType -ne 'USB' -or $d.IsBoot -or $d.IsSystem -or $d.IsClustered -or $d.IsReadOnly -or $d.IsOffline -or $d.Size -ne $e.size -or ([string]$d.SerialNumber).Trim() -cne $e.serial -or ([string]$d.UniqueId).Trim() -cne $e.unique_id) {throw 'USB identity or protection check failed'};
        }
        Assert-Target;
        Clear-Disk -Number $e.number -RemoveData -RemoveOEM -Confirm:$false;
        Assert-Target;Initialize-Disk -Number $e.number -PartitionStyle GPT|Out-Null;
        Assert-Target;$efi=New-Partition -DiskNumber $e.number -Offset 1MB -Size 1GB -GptType '{c12a7328-f81f-11d2-ba4b-00a0c93ec93b}' -AssignDriveLetter;
        Assert-Target;$efi|Format-Volume -FileSystem FAT32 -NewFileSystemLabel AXENTRA_EFI -Force -Confirm:$false|Out-Null;
        Assert-Target;$data=New-Partition -DiskNumber $e.number -UseMaximumSize -AssignDriveLetter;
        Assert-Target;$data|Format-Volume -FileSystem NTFS -NewFileSystemLabel AXENTRA_OS -Force -Confirm:$false|Out-Null;
        if (!$efi.DriveLetter -or !$data.DriveLetter) {throw 'Windows did not assign partition mount paths'};
        @{boot=([string]$efi.DriveLetter+':\');data=([string]$data.DriveLetter+':\')}|ConvertTo-Json -Compress
        "#;
        let v: Value = serde_json::from_str(&ps(&script)?)?;
        Ok(Volumes {
            boot: PathBuf::from(v["boot"].as_str().context("No EFI path")?),
            data: PathBuf::from(v["data"].as_str().context("No data path")?),
            mounted: vec![],
            _lock: None,
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        for tool in [
            "parted",
            "mkfs.vfat",
            "mkfs.ntfs",
            "mount",
            "umount",
            "udevadm",
            "blockdev",
        ] {
            ensure!(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                    .any(|p| p.join(tool).is_file()),
                "Missing utility {tool}. Install the package dependencies first."
            );
        }
        unmount(&d)?;
        revalidate(&d, 0)?;
        let lock = OpenOptions::new().read(true).write(true).open(&d.path)?;
        ensure!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "Device is in use"
        );
        cmd(
            "parted",
            &[
                "--script",
                &d.path,
                "mklabel",
                "gpt",
                "mkpart",
                "ESP",
                "fat32",
                "1MiB",
                "1025MiB",
                "set",
                "1",
                "esp",
                "on",
                "mkpart",
                "AXENTRA_OS",
                "ntfs",
                "1025MiB",
                "100%",
            ],
        )?;
        cmd("blockdev", &["--rereadpt", &d.path])?;
        cmd("udevadm", &["settle"])?;
        revalidate(&d, 0)?;
        let suffix = if d.path.ends_with(|c: char| c.is_ascii_digit()) {
            "p"
        } else {
            ""
        };
        let p1 = format!("{}{suffix}1", d.path);
        let p2 = format!("{}{suffix}2", d.path);
        cmd("mkfs.vfat", &["-F", "32", "-n", "AXENTRA_EFI", &p1])?;
        revalidate(&d, 0)?;
        cmd("mkfs.ntfs", &["-Q", "-L", "AXENTRA_OS", &p2])?;
        let mut v = Volumes {
            boot: scratch.join("efi"),
            data: scratch.join("data"),
            mounted: vec![],
            _lock: Some(lock),
        };
        for (part, root, fs) in [(&p1, &v.boot, "vfat"), (&p2, &v.data, "ntfs-3g")] {
            std::fs::create_dir(root)?;
            cmd(
                "mount",
                &[
                    "-t",
                    fs,
                    "-o",
                    "nodev,nosuid,noexec",
                    part,
                    root.to_str().context("Invalid mount path")?,
                ],
            )?;
            v.mounted.push(root.clone());
        }
        Ok(v)
    }
}
pub fn flush(root: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        ps(&format!(
            "$p={};Get-Volume -FilePath $p|Write-VolumeCache",
            ps_data(&root.to_string_lossy())?
        ))?;
    }
    #[cfg(unix)]
    {
        let _ = root;
        unsafe {
            libc::sync();
        }
    }
    Ok(())
}

pub fn run_elevated(request: &Path) -> Result<()> {
    let executable = std::env::current_exe()?;
    if admin() {
        let status = Command::new(executable)
            .arg("--worker")
            .arg(request)
            .status()?;
        ensure!(status.success(), "Device operation did not complete");
        return Ok(());
    }
    #[cfg(unix)]
    {
        let status = Command::new("pkexec")
            .arg(executable)
            .arg("--worker")
            .arg(request)
            .status()?;
        ensure!(
            status.success(),
            "Device access cancelled or operation failed"
        );
        Ok(())
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
            UI::{
                Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
                WindowsAndMessaging::SW_HIDE,
            },
        };
        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(Some(0)).collect()
        }
        let file = wide(&executable.to_string_lossy());
        let args = wide(&format!("--worker \"{}\"", request.to_string_lossy()));
        let verb = wide("runas");
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = args.as_ptr();
        info.nShow = SW_HIDE;
        ensure!(
            unsafe { ShellExecuteExW(&mut info) } != 0,
            "Device access was not authorized; no operation started"
        );
        unsafe {
            WaitForSingleObject(info.hProcess, INFINITE);
        }
        let mut exit = 1;
        unsafe {
            GetExitCodeProcess(info.hProcess, &mut exit);
            CloseHandle(info.hProcess);
        }
        ensure!(exit == 0, "Device worker did not complete successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn usb() -> Device {
        Device {
            path: "/dev/sdb".into(),
            bus: "usb".into(),
            serial: "abc123".into(),
            unique_id: "stable".into(),
            size: 32 * 1024 * 1024 * 1024,
            sector: 512,
            model: "USB".into(),
            ..Default::default()
        }
    }
    #[test]
    fn usb_allowed() {
        assert!(usb().blocked().is_none());
    }
    #[test]
    fn protect_system() {
        let mut d = usb();
        d.system = true;
        assert!(d.blocked().is_some());
    }
    #[test]
    fn protect_internal() {
        let mut d = usb();
        d.bus = "nvme".into();
        assert!(d.blocked().is_some());
    }
    #[test]
    fn protect_no_identity() {
        let mut d = usb();
        d.serial.clear();
        assert!(d.blocked().is_some());
    }
    #[test]
    fn protect_readonly() {
        let mut d = usb();
        d.read_only = true;
        assert!(d.blocked().is_some());
    }
    #[test]
    fn reject_replug() {
        let a = usb();
        let mut b = a.clone();
        b.serial = "different".into();
        assert!(validate_snapshot(&a, &b, 0).is_err());
    }
    #[test]
    fn reject_capacity_change() {
        let a = usb();
        let mut b = a.clone();
        b.size /= 2;
        assert!(validate_snapshot(&a, &b, 0).is_err());
    }
    #[test]
    fn reject_too_small() {
        let a = usb();
        assert!(validate_snapshot(&a, &a, a.size + 1).is_err());
    }
    #[test]
    fn system_rechecked() {
        let a = usb();
        let mut b = a.clone();
        b.system = true;
        assert!(validate_snapshot(&a, &b, 0).is_err());
    }
    #[test]
    fn token_contains_serial() {
        assert_eq!(usb().token(), "ERASE abc123");
    }
    #[test]
    fn linux_boot_usb_protected() {
        let doc = json!({"blockdevices":[{"type":"disk","path":"/dev/sdb","size":9999999,"serial":"s","tran":"usb","ro":false,"children":[{"type":"part","path":"/dev/sdb1","size":1024,"mountpoints":["/boot/efi"]}]}]});
        assert!(parse_lsblk(&doc).unwrap()[0].system);
    }
    #[test]
    fn linux_nested_lvm_protected() {
        let doc = json!({"blockdevices":[{"type":"disk","size":9999999,"ro":false,"children":[{"type":"crypt","children":[{"type":"lvm","mountpoints":["/"]}]}]}]});
        assert!(parse_lsblk(&doc).unwrap()[0].system);
    }
}
