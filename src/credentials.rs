//! Provider credentials live in one native credential-store item. A single
//! update avoids partial changes across providers. Error messages deliberately
//! exclude platform error payloads, which can contain raw secret bytes.
use crate::settings::AppSettings;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderKeys {
    openrouter: String,
    openai: String,
    groq: String,
}

impl ProviderKeys {
    fn from_settings(settings: &AppSettings) -> Self {
        Self {
            openrouter: settings.openrouter_api_key.clone(),
            openai: settings.openai_api_key.clone(),
            groq: settings.groq_api_key.clone(),
        }
    }
    fn apply(self, settings: &mut AppSettings) {
        settings.openrouter_api_key = self.openrouter;
        settings.openai_api_key = self.openai;
        settings.groq_api_key = self.groq;
    }
    fn is_empty(&self) -> bool {
        self.openrouter.is_empty() && self.openai.is_empty() && self.groq.is_empty()
    }
    fn validate(&self) -> Result<(), String> {
        if [&self.openrouter, &self.openai, &self.groq]
            .iter()
            .any(|key| key.len() > 1024 || key.contains(['\n', '\r', '\0']))
        {
            Err("An API key is too long or contains a line break. Paste only the key.".to_owned())
        } else {
            Ok(())
        }
    }
}

fn entry() -> Result<keyring::v1::Entry, String> {
    keyring::v1::Entry::new("org.lawpdf.LawPDF", "provider-api-keys-v1").map_err(|_| unavailable())
}

fn unavailable() -> String {
    "Could not access the system credential store. Unlock Keychain or Windows Credential Manager, then save your settings again. Existing credentials were kept.".to_owned()
}

fn read(entry: &keyring::v1::Entry) -> Result<Option<ProviderKeys>, String> {
    match entry.get_password() {
        Ok(secret) => {
            if secret.len() > 8192 {
                return Err(
                    "The stored credential record is invalid. Enter your keys again in Settings."
                        .to_owned(),
                );
            }
            let keys: ProviderKeys = serde_json::from_str(&secret).map_err(|_| {
                "The stored credential record is unreadable. Enter your keys again in Settings."
                    .to_owned()
            })?;
            keys.validate()?;
            Ok(Some(keys))
        }
        Err(keyring::v1::Error::NoEntry) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}

fn write(entry: &keyring::v1::Entry, keys: &ProviderKeys) -> Result<(), String> {
    keys.validate()?;
    let encoded =
        serde_json::to_string(keys).map_err(|_| "Could not encode credentials.".to_owned())?;
    entry.set_password(&encoded).map_err(|_| unavailable())?;
    if read(entry)?.as_ref() != Some(keys) {
        return Err(
            "Secure credential storage could not be confirmed. Existing settings were kept."
                .to_owned(),
        );
    }
    Ok(())
}

/// Load the native store first; a previous successful migration is authoritative
/// even if removing legacy plaintext was interrupted. Only after verified
/// storage does the caller remove keys from the settings file.
pub fn load_and_migrate(settings: &mut AppSettings) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let entry = entry()?;
    if let Some(keys) = read(&entry)? {
        keys.apply(settings);
    } else {
        let keys = ProviderKeys::from_settings(settings);
        if !keys.is_empty() {
            write(&entry, &keys)?;
        }
    }
    Ok(())
}

pub fn save(settings: &AppSettings) -> Result<(), String> {
    if cfg!(test) {
        return Err("Tests must use an isolated credential store.".to_owned());
    }
    // An empty record is intentional: it prevents interrupted migration from
    // resurrecting a key that the user removed.
    write(&entry()?, &ProviderKeys::from_settings(settings))
}

#[cfg(feature = "devtools")]
pub fn verify_native_store() -> Result<(), String> {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Could not create QA identity")?.as_nanos();
    let account = format!("qa-{}-{nonce}", std::process::id());
    struct QaEntry(keyring::v1::Entry);
    impl Drop for QaEntry {
        fn drop(&mut self) { let _ = self.0.delete_credential(); }
    }
    // This separate service never reads or overwrites the provider-key entry.
    let entry = QaEntry(keyring::v1::Entry::new("org.lawpdf.LawPDF.release-qa", &account)
        .map_err(|_| unavailable())?);
    let keys = ProviderKeys { openrouter: "non-secret-release-qa".to_owned(), ..Default::default() };
    write(&entry.0, &keys)?;
    write(&entry.0, &ProviderKeys::default())?;
    entry.0.delete_credential().map_err(|_| "Could not delete the disposable credential-store QA entry")?;
    if read(&entry.0)?.is_some() { return Err("The disposable QA credential was not deleted".to_owned()); }
    println!("Native credential write, read, clear and deletion passed using a disposable service");
    Ok(())
}
