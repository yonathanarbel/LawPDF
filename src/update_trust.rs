//! The release service is a transport, not a signing authority. Only exact
//! manifest bytes signed by the pinned maintainer key authorize installation.
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const MANIFEST_NAME: &str = "UPDATE-MANIFEST.json";
pub const SIGNATURE_NAME: &str = "UPDATE-MANIFEST.sig";
pub const MAX_MANIFEST_BYTES: usize = 262144;
pub const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const PUBLIC_KEY: &str = include_str!("../packaging/update-public-key.hex");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema: u32,
    pub purpose: String,
    pub repository: String,
    pub version: String,
    pub commit: String,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAsset {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

pub fn verify(manifest: &str, signature: &str) -> Result<ReleaseManifest, String> {
    verify_with_key(manifest, signature, PUBLIC_KEY.trim())
}

fn verify_with_key(manifest: &str, signature: &str, key: &str) -> Result<ReleaseManifest, String> {
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err("Update manifest is too large.".to_owned());
    }
    let key = VerifyingKey::from_bytes(&decode_hex::<32>(key)?)
        .map_err(|_| "Invalid update verification key.".to_owned())?;
    let signature = Signature::try_from(decode_hex::<64>(signature.trim())?.as_slice())
        .map_err(|_| "Invalid release signature.".to_owned())?;
    key.verify_strict(manifest.as_bytes(), &signature)
        .map_err(|_| {
            "The release signature is not trusted. The update was not installed.".to_owned()
        })?;
    let release: ReleaseManifest = serde_json::from_str(manifest)
        .map_err(|_| "The signed update manifest is invalid.".to_owned())?;
    if release.schema != 1
        || release.purpose != "lawpdf-update-v1"
        || release.repository != "yonathanarbel/LawPDF"
        || version(&release.version).is_none()
        || release.commit.len() != 40
        || !release.commit.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("The signed manifest does not describe a supported LawPDF release.".to_owned());
    }
    if release.assets.is_empty() || release.assets.len() > 3 {
        return Err("Invalid update artifact list.".to_owned());
    }
    let mut names = std::collections::HashSet::new();
    for asset in &release.assets {
        if !matches!(
            asset.name.as_str(),
            "LawPDFSetup-x64.exe" | "LawPDF-windows-portable-x64.zip" | "LawPDF-macos.zip"
        ) || !names.insert(&asset.name)
            || asset.bytes == 0
            || asset.bytes > MAX_PACKAGE_BYTES
            || decode_hex::<32>(&asset.sha256).is_err()
        {
            return Err("The signed manifest contains an invalid artifact.".to_owned());
        }
    }
    Ok(release)
}

pub fn version(value: &str) -> Option<[u64; 3]> {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    let mut version = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty()
            || part.len() > 10
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        version[index] = part.parse().ok()?;
    }
    Some(version)
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid update signature or digest encoding.".to_owned());
    }
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "Invalid hexadecimal data.".to_owned())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    fn manifest() -> ReleaseManifest {
        ReleaseManifest {
            schema: 1,
            purpose: "lawpdf-update-v1".to_owned(),
            repository: "yonathanarbel/LawPDF".to_owned(),
            version: "0.2.99".to_owned(),
            commit: "a".repeat(40),
            assets: vec![ReleaseAsset {
                name: "LawPDF-macos.zip".to_owned(),
                sha256: "b".repeat(64),
                bytes: 1024,
            }],
        }
    }
    fn signed(value: &ReleaseManifest) -> (String, String, String) {
        let key = SigningKey::from_bytes(&[42; 32]);
        let json = serde_json::to_string(value).unwrap();
        let signature = hex(&key.sign(json.as_bytes()).to_bytes());
        (json, signature, hex(key.verifying_key().as_bytes()))
    }
    #[test]
    fn exact_manifest_signature_is_required() {
        let (json, signature, key) = signed(&manifest());
        assert!(verify_with_key(&json, &signature, &key).is_ok());
        assert!(verify_with_key(&(json.clone() + " "), &signature, &key).is_err());
        assert!(verify_with_key(&json.replace("1024", "1025"), &signature, &key).is_err());
        assert!(verify(&json, &signature).is_err()); // test key is never the production authority
    }
    #[test]
    fn even_signed_manifests_reject_wrong_identity_and_artifact_paths() {
        for change in 0..3 {
            let mut value = manifest();
            match change {
                0 => value.repository = "attacker/LawPDF".to_owned(),
                1 => value.assets[0].name = "../LawPDF-macos.zip".to_owned(),
                _ => value.assets.push(value.assets[0].clone()),
            }
            let (json, signature, key) = signed(&value);
            assert!(verify_with_key(&json, &signature, &key).is_err());
        }
    }
    #[test]
    fn version_parser_has_no_ambiguous_or_prerelease_forms() {
        assert_eq!(version("12.34.56"), Some([12, 34, 56]));
        for invalid in [
            "0.3",
            "v0.3.0",
            "0.03.0",
            "0.3.0-beta",
            "0.3.0/../x",
            "0.3.0\n",
        ] {
            assert!(version(invalid).is_none());
        }
    }
}
