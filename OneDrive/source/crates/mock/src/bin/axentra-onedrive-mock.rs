//! A local fake OneDrive with sample files, for trying Axentra OneDrive without an
//! account: `axentra-onedrive-mock [ADDR]`, then point graph_base/authority at it.

use axentra_onedrive_mock::MockServer;

#[tokio::main]
async fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8765".into());
    let m = MockServer::start_on(&addr).await;
    for dir in ["Documents", "Pictures", "Desktop", "Music"] {
        m.add_folder("/", dir);
    }
    m.add_folder("/Pictures", "Camera Roll");
    m.add_file(
        "/",
        "Getting started with OneDrive.pdf",
        &vec![7u8; 1_150_000],
    );
    m.add_file("/Documents", "Budget 2026.xlsx", &vec![1u8; 48_213]);
    m.add_file("/Documents", "Resume.docx", &vec![2u8; 23_904]);
    m.add_file(
        "/Documents",
        "Meeting notes.txt",
        b"Agenda\n- Linux client demo\n",
    );
    m.add_file(
        "/Pictures/Camera Roll",
        "IMG_2041.jpg",
        &vec![3u8; 2_400_000],
    );
    m.add_file(
        "/Pictures/Camera Roll",
        "IMG_2042.jpg",
        &vec![4u8; 2_100_000],
    );
    m.add_file(
        "/Pictures",
        "Screenshot 2026-10-12.png",
        &vec![5u8; 380_000],
    );
    println!("Mock OneDrive listening on {}", m.origin);
    println!("Use it with these settings in ~/.config/axentra/onedrive/config.toml:");
    println!(
        "  client_id = \"mock\"\n  graph_base = \"{}\"\n  authority = \"{}\"",
        m.graph_base, m.origin
    );
    println!(
        "and write this to ~/.local/share/axentra/onedrive/tokens.json:\n  {}",
        m.tokens_json(false)
    );
    tokio::signal::ctrl_c().await.ok();
}
