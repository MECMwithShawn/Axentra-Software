use super::*;
use std::io::{Read,Write};
use std::net::TcpListener;
fn server(response:Vec<u8>)->(String,std::thread::JoinHandle<()>) {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let url=format!("http://{}/",listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let thread=std::thread::spawn(move|| {
        let start=std::time::Instant::now();
        loop {
            match listener.accept() {
                Ok((mut s,_))=>{
                    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut request=[0;4096]; let _=s.read(&mut request);
                    s.write_all(&response).unwrap(); break;
                },
                Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>{
                    assert!(start.elapsed()<Duration::from_secs(10),"local test listener timed out");
                    std::thread::sleep(Duration::from_millis(10));
                },
                Err(e)=>panic!("{e}")
            }
        }
    });
    (url,thread)
}
fn ok(body:&[u8],headers:&str)->Vec<u8> {
    let mut out=format!("HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n",body.len()).into_bytes();
    out.extend_from_slice(body); out
}
#[test]
fn audit_page_icon_href_can_fetch_loopback() {
    let (url,thread)=server(ok(b"synthetic-local-response",""));
    let candidates=icon_links(&format!("<link rel='icon' href='{url}'>"),"https://public.example/");
    assert_eq!(candidates.len(),1);
    let (bytes,_)=get_bytes(&agent(),&candidates[0].url,MAX_IMAGE).unwrap();
    thread.join().unwrap(); assert_eq!(bytes,b"synthetic-local-response");
    println!("CONFIRMED: public-site icon link fetched loopback successfully.");
}
#[test]
fn audit_redirect_can_fetch_loopback() {
    let (target,t)=server(ok(b"redirect-target",""));
    let redirect=format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let (first,f)=server(redirect.into_bytes());
    let (bytes,final_url)=get_bytes(&agent(),&first,MAX_IMAGE).unwrap();
    f.join().unwrap(); t.join().unwrap(); assert_eq!(bytes,b"redirect-target"); assert_eq!(final_url,target);
    println!("CONFIRMED: redirect targets receive no address validation.");
}
#[test]
fn audit_gzip_decoding_bypasses_body_limit() {
    let compressed=std::fs::read(std::env::var("AUDIT_GZIP").unwrap()).unwrap();
    assert!(compressed.len()<MAX_IMAGE as usize);
    let (url,t)=server(ok(&compressed,"Content-Encoding: gzip\r\n"));
    let (bytes,_)=get_bytes(&agent(),&url,MAX_IMAGE).unwrap();
    t.join().unwrap(); assert_eq!(bytes.len(),8*1024*1024); assert!(bytes.len()>MAX_IMAGE as usize);
    println!("CONFIRMED: {} compressed bytes expanded to {} bytes past the {}-byte image cap.",compressed.len(),bytes.len(),MAX_IMAGE);
}
#[test]
fn audit_svg_reads_local_image() {
    let path=std::env::temp_dir().join(format!("audit-image-{}.png",uuid::Uuid::new_v4()));
    image::RgbaImage::from_pixel(32,32,image::Rgba([255,0,0,255])).save(&path).unwrap();
    let svg=format!("<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='64' height='64'><image width='64' height='64' xlink:href='{}'/></svg>",path.display());
    let png=to_png(svg.as_bytes()).unwrap();
    let decoded=image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(decoded.get_pixel(32,32).0,[255,0,0,255]);
    println!("CONFIRMED: untrusted SVG loads an absolute local image via default resolver.");
}
#[test]
fn audit_icon_size_overflows_score() {
    let result=std::panic::catch_unwind(||icon_links("<link rel='apple-touch-icon' href='/x.png' sizes='4294967295x1'>","https://example.com/"));
    assert!(result.is_err());
    println!("CONFIRMED: untrusted apple-touch-icon size panics in debug; release wraps its ranking score.");
}
#[test]
fn audit_chunked_body_limit_is_enforced() {
    let response=b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n8\r\n12345678\r\n0\r\n\r\n".to_vec();
    let (url,t)=server(response); assert!(get_bytes(&agent(),&url,4).is_err()); t.join().unwrap();
}

