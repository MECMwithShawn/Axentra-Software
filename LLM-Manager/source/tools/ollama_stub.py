"""Stub Ollama on loopback: tags/ps/version + load/unload via /api/generate."""
import json, sys, time, threading
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
G=1024**3
MODELS=[
 ("deepseek-r1:14b",9.0*G,"qwen2","14.8B",131072),
 ("qwen3:8b",5.2*G,"qwen3","8.2B",40960),
 ("qwen3:14b",9.3*G,"qwen3","14.8B",40960),
 ("phi4-mini:latest",2.5*G,"phi3","3.8B",131072),
 ("hf.co/example/Very-Long-Experimental-Model-Name-Instruct-GGUF:Q4_K_M-extra-long-tag",2.5*G,"llama","70.6B",262144),
]
resident={}  # name -> True
lock=threading.Lock()
MODE=sys.argv[2] if len(sys.argv)>2 else "ok"
class H(BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def send(self,obj,code=200):
        b=json.dumps(obj).encode(); self.send_response(code)
        self.send_header("Content-Type","application/json"); self.send_header("Content-Length",str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_GET(self):
        if self.path=="/api/tags":
            self.send({"models":[{"name":n,"size":int(s),"details":{"family":f,"parameter_size":p,"quantization_level":"Q4_K_M","context_length":c}} for n,s,f,p,c in MODELS]})
        elif self.path=="/api/ps":
            with lock:
                self.send({"models":[{"name":n,"size":int(s*1.18),"size_vram":int(s*1.1),"context_length":4096,"runner":"llamacpp","expires_at":"2026-11-17T10:05:00Z"} for n,s,*_ in MODELS if n in resident]})
        elif self.path=="/api/version": self.send({"version":"0.40.14-stub"})
        else: self.send({"error":"not found"},404)
    def do_POST(self):
        body=json.loads(self.rfile.read(int(self.headers.get("Content-Length",0))))
        name=body["model"].removesuffix(":local"); load=body["keep_alive"]!=0
        time.sleep(1.5)
        if MODE=="fail": return self.send({"error":f"stub refused {name}"},500)
        with lock:
            if load: resident[name]=True
            else: resident.pop(name,None)
        self.send({"done":True,"done_reason":"load" if load else "unload"})
ThreadingHTTPServer(("127.0.0.1",int(sys.argv[1])),H).serve_forever()
