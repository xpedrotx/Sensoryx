//! Atualização do app: baixa o instalador de um release do GitHub, confere o SHA-256 publicado
//! e só então o executa. A consulta à API do GitHub é feita pela tela de configurações.

use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

/// Repositório dos releases no formato `dono/nome`. Se estiver vazio, a aba de atualização
/// avisa que ainda não está configurada.
pub const UPDATE_REPO: &str = "xpedrotx/Sensoryx";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Eventos que a tela de configurações mostra ao usuário.
pub enum Progress {
    Percent(u32),
    Verifying,
    Installing,
    Failed(String),
}

/// A URL precisa ser um arquivo de release do repositório configurado, e o nome precisa ter a forma
/// `SensoryxSetup-<versão>.exe` (evita baixar/executar qualquer outra coisa).
pub fn validate_url(url: &str, repo: &str) -> Result<String, String> {
    if repo.is_empty() {
        return Err("repositório de atualização não configurado".into());
    }
    let prefix = format!("https://github.com/{repo}/releases/download/");
    let rest = url.strip_prefix(&prefix).ok_or("endereço de download não pertence ao repositório oficial")?;
    let name = rest.rsplit('/').next().unwrap_or("");
    let ok = name.starts_with("SensoryxSetup-")
        && name.ends_with(".exe")
        && name.len() < 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'));
    if !ok || rest.contains("..") {
        return Err("nome de arquivo inesperado".into());
    }
    Ok(name.to_string())
}

pub fn sha256_hex(path: &PathBuf) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Baixa (com `curl.exe`, que acompanha o Windows 10/11), confere o hash e executa o instalador.
pub fn download_and_install(url: &str, expected_sha256: &str, size: u64, tx: &Sender<Progress>) {
    let name = match validate_url(url, UPDATE_REPO) {
        Ok(n) => n,
        Err(e) => return fail(tx, &e),
    };
    if expected_sha256.len() != 64 || !expected_sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return fail(tx, "hash SHA-256 inválido");
    }
    let dir = std::env::temp_dir().join("SensoryxUpdate");
    let _ = fs::remove_dir_all(&dir);
    if fs::create_dir_all(&dir).is_err() {
        return fail(tx, "não foi possível criar a pasta temporária");
    }
    let file = dir.join(&name);

    let child = Command::new("curl.exe")
        .args(["--location", "--fail", "--silent", "--show-error", "--proto", "=https", "--max-time", "600", "-o"])
        .arg(&file)
        .arg(url)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return fail(tx, &format!("curl indisponível: {e}")),
    };

    let t0 = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if size > 0 {
                    let got = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                    let _ = tx.send(Progress::Percent(((got * 100) / size).min(99) as u32));
                }
                if t0.elapsed() > Duration::from_secs(660) {
                    let _ = child.kill();
                    return fail(tx, "tempo esgotado no download");
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return fail(tx, &e.to_string()),
        }
    };
    if !status.success() {
        let _ = fs::remove_file(&file);
        return fail(tx, "o download falhou");
    }
    let _ = tx.send(Progress::Percent(100));

    let _ = tx.send(Progress::Verifying);
    match sha256_hex(&file) {
        Ok(h) if h.eq_ignore_ascii_case(expected_sha256) => {}
        Ok(_) => {
            let _ = fs::remove_file(&file);
            return fail(tx, "o hash SHA-256 não confere: o arquivo foi descartado por segurança");
        }
        Err(e) => return fail(tx, &format!("não foi possível ler o arquivo: {e}")),
    }

    let _ = tx.send(Progress::Installing);
    // /SILENT: sem assistente; o instalador fecha o Sensoryx em execução e o reabre ao terminar.
    if let Err(e) = Command::new(&file).args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]).spawn() {
        fail(tx, &format!("não foi possível iniciar o instalador: {e}"));
    }
}

fn fail(tx: &Sender<Progress>, msg: &str) {
    let _ = tx.send(Progress::Failed(msg.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_official_release_asset() {
        let u = "https://github.com/dono/Sensoryx/releases/download/v0.2.0/SensoryxSetup-0.2.0.exe";
        assert_eq!(validate_url(u, "dono/Sensoryx").unwrap(), "SensoryxSetup-0.2.0.exe");
    }

    #[test]
    fn rejects_other_hosts_repos_and_names() {
        let ok = "https://github.com/dono/Sensoryx/releases/download/v1/SensoryxSetup-1.0.exe";
        assert!(validate_url(ok, "").is_err()); // sem repositório configurado
        assert!(validate_url(ok, "outro/Repo").is_err());
        assert!(validate_url("http://github.com/dono/Sensoryx/releases/download/v1/SensoryxSetup-1.0.exe", "dono/Sensoryx").is_err());
        assert!(validate_url("https://evil.com/dono/Sensoryx/releases/download/v1/SensoryxSetup-1.0.exe", "dono/Sensoryx").is_err());
        assert!(validate_url("https://github.com/dono/Sensoryx/releases/download/v1/malware.exe", "dono/Sensoryx").is_err());
        assert!(validate_url("https://github.com/dono/Sensoryx/releases/download/v1/SensoryxSetup-1.0.exe?x=1", "dono/Sensoryx").is_err());
        assert!(validate_url("https://github.com/dono/Sensoryx/releases/download/../SensoryxSetup-1.0.exe", "dono/Sensoryx").is_err());
    }

    #[test]
    fn sha256_matches_known_vector() {
        let p = std::env::temp_dir().join("sensoryx_sha_test.bin");
        fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_hex(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = fs::remove_file(&p);
    }
}
