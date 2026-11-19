use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use minisign::{KeyPair, SecretKey, SecretKeyBox};
use serde_json::Value;

fn usage() -> ! {
    eprintln!(
        "Usage:
  zyplus-release-signer sign --file <path> --out <sig_path> [--key-file <path>] [--key-env <var>] [--password-env <var>] [--trusted-comment <text>]
  zyplus-release-signer verify --file <path> --sig <sig_path> --pubkey <b64_or_path>
  zyplus-release-signer keygen --out-key <path> --out-pub <path> [--password <pw>] [--raw-key]
  zyplus-release-signer check-manifest --manifest <latest.json> --file <archive> --pubkey <b64_or_path>"
    );
    std::process::exit(2);
}

fn next_val(args: &mut std::vec::IntoIter<String>, flag: &str) -> String {
    args.next().unwrap_or_else(|| {
        eprintln!("Missing value for {flag}");
        std::process::exit(2);
    })
}

fn decode_key_text(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Signing private key is empty".to_string());
    }
    let normalized = trimmed.replace("\r\n", "\n");
    if normalized.starts_with("untrusted comment:") {
        return Ok(normalized);
    }
    let bytes = STANDARD
        .decode(normalized.lines().map(str::trim).collect::<String>())
        .map_err(|e| format!("Private key is neither minisign text nor valid base64: {e}"))?;
    let text =
        String::from_utf8(bytes).map_err(|e| format!("Decoded private key is not UTF-8: {e}"))?;
    let text = text.replace("\r\n", "\n");
    if !text.starts_with("untrusted comment:") {
        return Err("Decoded private key does not start with 'untrusted comment:'".to_string());
    }
    Ok(text)
}

fn load_secret_key(
    key_file: Option<&Path>,
    key_env: &str,
    password_env: &str,
) -> Result<SecretKey, String> {
    let raw_key = match key_file {
        Some(path) => fs::read_to_string(path)
            .map_err(|e| format!("Could not read key file {}: {e}", path.display()))?,
        None => env::var(key_env)
            .map_err(|_| format!("Missing signing secret environment variable {key_env}"))?,
    };
    let box_text = decode_key_text(&raw_key)?;
    let sk_box = SecretKeyBox::from_string(&box_text)
        .map_err(|e| format!("Invalid minisign secret key box: {e}"))?;

    match SecretKey::from_unencrypted_box(sk_box.clone()) {
        Ok(sk) => Ok(sk),
        Err(_) => {
            let password = env::var(password_env)
                .map_err(|_| format!("Signing key is encrypted, but {password_env} is not set"))?;
            SecretKey::from_box(sk_box, Some(password))
                .map_err(|e| format!("Could not decrypt minisign secret key: {e}"))
        }
    }
}

fn read_pubkey_text(pubkey_arg: &str) -> Result<String, String> {
    let path = Path::new(pubkey_arg);
    let raw = if path.is_file() {
        fs::read_to_string(path)
            .map_err(|e| format!("Could not read public key {}: {e}", path.display()))?
    } else {
        pubkey_arg.to_string()
    };
    let trimmed = raw.trim();
    if trimmed.starts_with("untrusted comment:") {
        return Ok(trimmed.replace("\r\n", "\n"));
    }
    let bytes = STANDARD
        .decode(trimmed)
        .map_err(|e| format!("The update public key is invalid base64: {e}"))?;
    String::from_utf8(bytes)
        .map(|s| s.replace("\r\n", "\n"))
        .map_err(|e| format!("The update public key is not UTF-8: {e}"))
}

fn verify_bytes_with_updater_logic(
    bytes: &[u8],
    signature_b64: &str,
    pubkey_arg: &str,
) -> Result<(), String> {
    let pubkey_text = read_pubkey_text(pubkey_arg)?;
    let pk = minisign_verify::PublicKey::decode(&pubkey_text)
        .map_err(|e| format!("The update public key is invalid: {e}"))?;
    let sig_bytes = STANDARD
        .decode(signature_b64.trim())
        .map_err(|e| format!("The update signature is invalid base64: {e}"))?;
    let sig_text = String::from_utf8(sig_bytes)
        .map_err(|e| format!("The update signature is not UTF-8: {e}"))?;
    let sig = minisign_verify::Signature::decode(&sig_text)
        .map_err(|e| format!("The update signature is invalid: {e}"))?;
    pk.verify(bytes, &sig, false)
        .map_err(|e| format!("Signature verification failed: {e}"))
}

fn cmd_sign(args: Vec<String>) -> Result<(), String> {
    let mut file: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut key_file: Option<PathBuf> = None;
    let mut key_env = "TAURI_SIGNING_PRIVATE_KEY".to_string();
    let mut password_env = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD".to_string();
    let mut trusted_comment: Option<String> = None;

    let mut it = args.into_iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--file" => file = Some(PathBuf::from(next_val(&mut it, "--file"))),
            "--out" => out = Some(PathBuf::from(next_val(&mut it, "--out"))),
            "--key-file" => key_file = Some(PathBuf::from(next_val(&mut it, "--key-file"))),
            "--key-env" => key_env = next_val(&mut it, "--key-env"),
            "--password-env" => password_env = next_val(&mut it, "--password-env"),
            "--trusted-comment" => trusted_comment = Some(next_val(&mut it, "--trusted-comment")),
            _ => {
                eprintln!("Unknown flag for sign: {flag}");
                usage();
            }
        }
    }

    let file = file.ok_or("--file is required")?;
    let out = out.ok_or("--out is required")?;
    let sk = load_secret_key(key_file.as_deref(), &key_env, &password_env)?;

    let bytes = fs::read(&file).map_err(|e| format!("Could not read {}: {e}", file.display()))?;
    let trusted = trusted_comment.unwrap_or_else(|| {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "artifact".to_string());
        format!("timestamp:{ts}\tfile:{name}")
    });

    let sig_box = minisign::sign(None, &sk, Cursor::new(&bytes), Some(&trusted), None)
        .map_err(|e| format!("Failed to sign {}: {e}", file.display()))?;
    let sig_b64 = STANDARD.encode(sig_box.into_string());

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
        }
    }
    fs::write(&out, format!("{sig_b64}\n"))
        .map_err(|e| format!("Could not write {}: {e}", out.display()))?;
    Ok(())
}

fn cmd_verify(args: Vec<String>) -> Result<(), String> {
    let mut file: Option<PathBuf> = None;
    let mut sig: Option<PathBuf> = None;
    let mut pubkey: Option<String> = None;

    let mut it = args.into_iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--file" => file = Some(PathBuf::from(next_val(&mut it, "--file"))),
            "--sig" => sig = Some(PathBuf::from(next_val(&mut it, "--sig"))),
            "--pubkey" => pubkey = Some(next_val(&mut it, "--pubkey")),
            _ => {
                eprintln!("Unknown flag for verify: {flag}");
                usage();
            }
        }
    }

    let file = file.ok_or("--file is required")?;
    let sig = sig.ok_or("--sig is required")?;
    let pubkey = pubkey.ok_or("--pubkey is required")?;

    let bytes = fs::read(&file).map_err(|e| format!("Could not read {}: {e}", file.display()))?;
    let sig_b64 =
        fs::read_to_string(&sig).map_err(|e| format!("Could not read {}: {e}", sig.display()))?;
    verify_bytes_with_updater_logic(&bytes, &sig_b64, &pubkey)
}

fn cmd_keygen(args: Vec<String>) -> Result<(), String> {
    let mut out_key: Option<PathBuf> = None;
    let mut out_pub: Option<PathBuf> = None;
    let mut password: Option<String> = None;
    let mut raw_key = false;

    let mut it = args.into_iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--out-key" => out_key = Some(PathBuf::from(next_val(&mut it, "--out-key"))),
            "--out-pub" => out_pub = Some(PathBuf::from(next_val(&mut it, "--out-pub"))),
            "--password" => password = Some(next_val(&mut it, "--password")),
            "--raw-key" => raw_key = true,
            _ => {
                eprintln!("Unknown flag for keygen: {flag}");
                usage();
            }
        }
    }

    let out_key = out_key.ok_or("--out-key is required")?;
    let out_pub = out_pub.ok_or("--out-pub is required")?;

    let kp = match password {
        Some(pw) => KeyPair::generate_encrypted_keypair(Some(pw))
            .map_err(|e| format!("Failed to generate encrypted keypair: {e}"))?,
        None => KeyPair::generate_unencrypted_keypair()
            .map_err(|e| format!("Failed to generate unencrypted keypair: {e}"))?,
    };
    let sk_text = kp
        .sk
        .to_box(None)
        .map_err(|e| format!("Failed to serialize secret key: {e}"))?
        .into_string();
    let pk_text = kp
        .pk
        .to_box()
        .map_err(|e| format!("Failed to serialize public key: {e}"))?
        .into_string();

    let key_out = if raw_key {
        sk_text
    } else {
        format!("{}\n", STANDARD.encode(sk_text))
    };
    let pub_out = format!("{}\n", STANDARD.encode(pk_text));

    fs::write(&out_key, key_out)
        .map_err(|e| format!("Could not write {}: {e}", out_key.display()))?;
    fs::write(&out_pub, pub_out)
        .map_err(|e| format!("Could not write {}: {e}", out_pub.display()))?;
    Ok(())
}

fn cmd_check_manifest(args: Vec<String>) -> Result<(), String> {
    let mut manifest_path: Option<PathBuf> = None;
    let mut file: Option<PathBuf> = None;
    let mut pubkey: Option<String> = None;

    let mut it = args.into_iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--manifest" => manifest_path = Some(PathBuf::from(next_val(&mut it, "--manifest"))),
            "--file" => file = Some(PathBuf::from(next_val(&mut it, "--file"))),
            "--pubkey" => pubkey = Some(next_val(&mut it, "--pubkey")),
            _ => {
                eprintln!("Unknown flag for check-manifest: {flag}");
                usage();
            }
        }
    }

    let manifest_path = manifest_path.ok_or("--manifest is required")?;
    let file = file.ok_or("--file is required")?;
    let pubkey = pubkey.ok_or("--pubkey is required")?;

    let body = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Could not read {}: {e}", manifest_path.display()))?;
    let json: Value =
        serde_json::from_str(&body).map_err(|e| format!("Manifest is not valid JSON: {e}"))?;

    let version = json
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("Manifest has no version")?;
    if !json.get("notes").is_some_and(Value::is_string) {
        return Err("Manifest 'notes' must be a string".to_string());
    }
    if json
        .get("pub_date")
        .and_then(Value::as_str)
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err("Manifest 'pub_date' must be a non-empty string".to_string());
    }

    let platforms = json
        .get("platforms")
        .and_then(Value::as_object)
        .ok_or("Manifest 'platforms' must be an object")?;
    let bytes = fs::read(&file).map_err(|e| format!("Could not read {}: {e}", file.display()))?;

    for key in ["darwin-aarch64", "darwin-x86_64"] {
        let entry = platforms
            .get(key)
            .ok_or_else(|| format!("Manifest missing platform {key}"))?;
        let url = entry
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Manifest {key} missing url"))?;
        if !url.starts_with("https://") && !url.starts_with("http://127.0.0.1:") {
            return Err(format!("Manifest {key} url is not HTTPS: {url}"));
        }
        let sig = entry
            .get("signature")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Manifest {key} missing signature"))?;
        verify_bytes_with_updater_logic(&bytes, sig, &pubkey)
            .map_err(|e| format!("Manifest {key} signature failed verification: {e}"))?;
    }

    println!("Verified manifest v{version} for darwin-aarch64 and darwin-x86_64");
    Ok(())
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        usage();
    }
    let cmd = args.remove(0);
    let result = match cmd.as_str() {
        "sign" => cmd_sign(args),
        "verify" => cmd_verify(args),
        "keygen" => cmd_keygen(args),
        "check-manifest" => cmd_check_manifest(args),
        _ => {
            eprintln!("Unknown subcommand: {cmd}");
            usage();
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("zyplus-release-signer: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_and_verifies_tauri_base64_and_raw_encrypted_keys() {
        let dir = env::temp_dir().join(format!("zyplus-signer-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let kp = KeyPair::generate_encrypted_keypair(Some("s3cret".to_string())).unwrap();
        let sk_text = kp.sk.to_box(None).unwrap().into_string();
        let pk_b64 = STANDARD.encode(kp.pk.to_box().unwrap().into_string());

        let data = b"zyplus archive payload";
        let artifact = dir.join("Zyplus.app.tar.gz");
        fs::write(&artifact, data).unwrap();

        for (label, key_content) in [
            ("base64", STANDARD.encode(&sk_text)),
            ("raw", sk_text.clone()),
        ] {
            let key_file = dir.join(format!("{label}.key"));
            fs::write(&key_file, key_content).unwrap();
            env::set_var("ZYPLUS_TEST_PW", "s3cret");
            let sk = load_secret_key(Some(&key_file), "UNUSED", "ZYPLUS_TEST_PW").unwrap();
            let sig_box = minisign::sign(
                None,
                &sk,
                Cursor::new(data),
                Some("timestamp:1700000000\tfile:Zyplus.app.tar.gz"),
                None,
            )
            .unwrap();
            let sig_b64 = STANDARD.encode(sig_box.into_string());
            assert_eq!(
                verify_bytes_with_updater_logic(data, &sig_b64, &pk_b64),
                Ok(())
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn signs_and_verifies_unencrypted_key() {
        let kp = KeyPair::generate_unencrypted_keypair().unwrap();
        let sk_b64 = STANDARD.encode(kp.sk.to_box(None).unwrap().into_string());
        let pk_b64 = STANDARD.encode(kp.pk.to_box().unwrap().into_string());

        let box_text = decode_key_text(&sk_b64).unwrap();
        let sk_box = SecretKeyBox::from_string(&box_text).unwrap();
        let sk = SecretKey::from_unencrypted_box(sk_box).unwrap();
        let data = b"unencrypted test";
        let sig_box = minisign::sign(None, &sk, Cursor::new(data), None, None).unwrap();
        let sig_b64 = STANDARD.encode(sig_box.into_string());
        assert_eq!(
            verify_bytes_with_updater_logic(data, &sig_b64, &pk_b64),
            Ok(())
        );
    }
}
