//! 开机自启：HKCU\Software\Microsoft\Windows\CurrentVersion\Run 下写一项 NxNote。
//! 启用时值 = `"C:\path\nxnote.exe" --hidden`（带 --hidden 让自启进入托盘隐藏态）。
//! 调 reg.exe 而非引入新依赖；用 CREATE_NO_WINDOW 避免一闪而过的黑框。

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE_NAME: &str = "NxNote";

#[cfg(windows)]
pub fn set_enabled(enable: bool) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if enable {
        let exe = std::env::current_exe()?;
        let exe_str = exe.to_string_lossy().to_string();
        let data = format!("\"{}\" --hidden", exe_str);
        let status = Command::new("reg")
            .args([
                "add",
                RUN_KEY,
                "/v",
                VALUE_NAME,
                "/t",
                "REG_SZ",
                "/d",
                &data,
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("reg add 退出码 {:?}", status.code()),
            ));
        }
    } else {
        // 已不存在时 reg delete 会返回非 0，但目标状态已达成 —— 不视作错误
        let _ = Command::new("reg")
            .args(["delete", RUN_KEY, "/v", VALUE_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set_enabled(_enable: bool) -> std::io::Result<()> {
    Ok(())
}
