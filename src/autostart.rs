//! Início automático com o Windows, por tarefa agendada com privilégio máximo
//! (o Sensoryx roda como administrador; a tarefa evita um pedido de UAC a cada login).

use std::{os::windows::process::CommandExt, process::Command};

const TASK: &str = "Sensoryx";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn is_enabled() -> bool {
    schtasks(&["/Query", "/TN", TASK])
}

/// Liga ou desliga o início automático. Devolve o estado final.
pub fn set(enable: bool) -> bool {
    if enable {
        if let Ok(exe) = std::env::current_exe() {
            let tr = format!("\"{}\"", exe.display());
            schtasks(&["/Create", "/TN", TASK, "/TR", &tr, "/SC", "ONLOGON", "/RL", "HIGHEST", "/F"]);
        }
    } else {
        schtasks(&["/Delete", "/TN", TASK, "/F"]);
    }
    is_enabled()
}
