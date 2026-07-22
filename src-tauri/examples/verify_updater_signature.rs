use base64::{engine::general_purpose::STANDARD, Engine as _};
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;
use std::{
    env,
    error::Error,
    ffi::OsString,
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("更新产物验证失败：{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let (artifact_path, signature_path, metadata_path) = arguments()?;
    let config_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let config: Value = serde_json::from_str(&fs::read_to_string(&config_path)?)?;
    let encoded_public_key = config
        .pointer("/plugins/updater/pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_data("tauri.conf.json 缺少 updater 公钥"))?;
    let public_key_text = String::from_utf8(STANDARD.decode(encoded_public_key)?)?;
    let encoded_signature = fs::read_to_string(&signature_path)?.trim().to_string();
    let signature_text = String::from_utf8(STANDARD.decode(&encoded_signature)?)?;

    let public_key = PublicKey::decode(&public_key_text)?;
    let signature = Signature::decode(&signature_text)?;
    let artifact = fs::read(&artifact_path)?;
    public_key.verify(&artifact, &signature, true)?;

    if let Some(metadata_path) = metadata_path {
        verify_metadata(&config, &artifact_path, &encoded_signature, &metadata_path)?;
    }

    println!("更新产物签名与发布元数据验证通过");
    Ok(())
}

fn arguments() -> Result<(PathBuf, PathBuf, Option<PathBuf>)> {
    let mut args = env::args_os().skip(1);
    let artifact = required_argument(&mut args, "缺少安装器路径")?;
    let signature = required_argument(&mut args, "缺少签名路径")?;
    let metadata = args.next().map(PathBuf::from);
    if args.next().is_some() {
        return Err(invalid_input(
            "用法：verify_updater_signature <安装器> <签名> [latest.json]",
        ));
    }
    Ok((artifact, signature, metadata))
}

fn required_argument(args: &mut impl Iterator<Item = OsString>, message: &str) -> Result<PathBuf> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| invalid_input(message))
}

fn verify_metadata(
    config: &Value,
    artifact_path: &Path,
    encoded_signature: &str,
    metadata_path: &Path,
) -> Result<()> {
    let metadata: Value = serde_json::from_str(&fs::read_to_string(metadata_path)?)?;
    let expected_version = config
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_data("tauri.conf.json 缺少版本号"))?;
    if metadata.get("version").and_then(Value::as_str) != Some(expected_version) {
        return Err(invalid_data("latest.json 版本与应用配置不一致"));
    }

    let artifact_name = artifact_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid_data("安装器文件名不是有效 UTF-8"))?;
    for platform in ["windows-x86_64", "windows-x86_64-nsis"] {
        let entry = metadata
            .pointer(&format!("/platforms/{platform}"))
            .ok_or_else(|| invalid_data(&format!("latest.json 缺少 {platform}")))?;
        let signature = entry
            .get("signature")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_data(&format!("{platform} 缺少签名")))?;
        if signature.trim() != encoded_signature {
            return Err(invalid_data(&format!("{platform} 签名与本地产物不一致")));
        }
        let url = entry
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_data(&format!("{platform} 缺少下载地址")))?;
        if !is_expected_download_url(url, artifact_name) {
            return Err(invalid_data(&format!(
                "{platform} 下载地址不指向当前安装器"
            )));
        }
    }
    Ok(())
}

fn is_expected_download_url(url: &str, artifact_name: &str) -> bool {
    const API_PREFIX: &str = "https://api.github.com/repos/mhdfy1988/todo/releases/assets/";
    if let Some(asset_id) = url.strip_prefix(API_PREFIX) {
        return !asset_id.is_empty()
            && asset_id.chars().all(|character| character.is_ascii_digit());
    }

    const RELEASE_PREFIX: &str = "https://github.com/mhdfy1988/todo/releases/download/";
    let ascii_suffix = artifact_name
        .find('_')
        .map(|index| &artifact_name[index..])
        .unwrap_or(artifact_name);
    url.starts_with(RELEASE_PREFIX) && url.ends_with(ascii_suffix)
}

fn invalid_input(message: &str) -> Box<dyn Error> {
    io::Error::new(ErrorKind::InvalidInput, message).into()
}

fn invalid_data(message: &str) -> Box<dyn Error> {
    io::Error::new(ErrorKind::InvalidData, message).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metadata_accepts_the_official_github_asset_api_url() {
        let directory =
            env::temp_dir().join(format!("todo-updater-metadata-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let artifact = directory.join("代办_0.1.0_x64-setup.exe");
        let metadata_path = directory.join("latest.json");
        let signature = "encoded-signature";
        let entry = json!({
            "signature": signature,
            "url": "https://api.github.com/repos/mhdfy1988/todo/releases/assets/485644281"
        });
        fs::write(
            &metadata_path,
            serde_json::to_vec(&json!({
                "version": "0.1.0",
                "platforms": {
                    "windows-x86_64": entry.clone(),
                    "windows-x86_64-nsis": entry
                }
            }))
            .unwrap(),
        )
        .unwrap();

        verify_metadata(
            &json!({ "version": "0.1.0" }),
            &artifact,
            signature,
            &metadata_path,
        )
        .unwrap();

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn metadata_rejects_an_asset_url_outside_the_repository() {
        assert!(!is_expected_download_url(
            "https://api.github.com/repos/other/todo/releases/assets/485644281",
            "代办_0.1.0_x64-setup.exe"
        ));
    }
}
