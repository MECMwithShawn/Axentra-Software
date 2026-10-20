pub mod device;
pub mod download;
pub mod image;
pub mod operation;
pub type Progress<'a> = dyn FnMut(&str, u64, u64) + 'a;
pub type Cancel<'a> = dyn Fn() -> bool + 'a;
pub const CHUNK: usize = 4 * 1024 * 1024;
pub fn check_cancel(cancel: &Cancel<'_>) -> anyhow::Result<()> {
    anyhow::ensure!(
        !cancel(),
        "Stopped. An interrupted write leaves incomplete USB media."
    );
    Ok(())
}

pub mod raid;
