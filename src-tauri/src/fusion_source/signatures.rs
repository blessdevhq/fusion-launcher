//! Source-index signatures bind the publisher's original wire object, before
//! URL resolution or schema normalization. A persisted public key is authority.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::schema::SourceIndex;

const DOMAIN: &[u8] = b"FusionSourceIndex.v1\0";
const PRIVATE_KEY_LIMIT: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedSourceIdentity {
    pub source_id: String,
    pub url: String,
    pub public_key: Option<String>,
}

pub trait SourceIdentityLookup: Send + Sync {
    /// Must include disconnected identities and match either ID or original URL.
    fn pinned_identity(
        &self,
        source_id: &str,
        index_url: &str,
    ) -> Result<Option<PinnedSourceIdentity>, FusionError>;
}

fn invalid(message: impl Into<String>) -> FusionError {
    FusionError::new(ErrorCode::InvalidSourceSignature, message)
}

/// serde_json normally discards duplicate object keys. Reject them recursively
/// so signers and consumers cannot disagree about the signed object.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ValueVisitor;
        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|value| UniqueValue(Value::Number(value)))
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.to_owned())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                self.visit_none()
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, UniqueValue(value))) =
                    map.next_entry::<String, UniqueValue>()?
                {
                    if values.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate JSON object key"));
                    }
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(ValueVisitor)
    }
}

pub fn parse_wire_index(json: &str) -> Result<Value, FusionError> {
    let document: UniqueValue = serde_json::from_str(json)
        .map_err(|_| invalid("The source index is not unambiguous JSON."))?;
    if !document.0.is_object() {
        return Err(invalid("The source index must be a JSON object."));
    }
    Ok(document.0)
}

pub fn source_id(document: &Value) -> Result<&str, FusionError> {
    document
        .pointer("/source_info/id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("The source index has no source identity."))
}

fn public_key(document: &Value) -> Result<Option<&str>, FusionError> {
    match document.pointer("/source_info/public_key") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(key)) => Ok(Some(key)),
        _ => Err(invalid("The source index public key is malformed.")),
    }
}

/// Compact serialization of recursively sorted keys. Array order, nulls, case,
/// and the original relative URL strings remain part of the signed payload.
pub fn signing_payload(document: &Value) -> Result<Vec<u8>, FusionError> {
    fn sorted(value: &Value) -> Result<Vec<u8>, FusionError> {
        match value {
            Value::Object(map) => {
                let ordered: BTreeMap<_, _> = map.iter().collect();
                let mut result = vec![b'{'];
                for (position, (key, value)) in ordered.into_iter().enumerate() {
                    if position != 0 {
                        result.push(b',');
                    }
                    result.extend(
                        serde_json::to_vec(key)
                            .map_err(|_| invalid("Cannot serialize signature key."))?,
                    );
                    result.push(b':');
                    result.extend(sorted(value)?);
                }
                result.push(b'}');
                Ok(result)
            }
            Value::Array(values) => {
                let mut result = vec![b'['];
                for (position, value) in values.iter().enumerate() {
                    if position != 0 {
                        result.push(b',');
                    }
                    result.extend(sorted(value)?);
                }
                result.push(b']');
                Ok(result)
            }
            _ => {
                serde_json::to_vec(value).map_err(|_| invalid("Cannot serialize signature value."))
            }
        }
    }
    let mut unsigned = document.clone();
    unsigned
        .as_object_mut()
        .ok_or_else(|| invalid("The source index must be an object."))?
        .remove("signature");
    let mut payload = DOMAIN.to_vec();
    payload.extend(sorted(&unsigned)?);
    Ok(payload)
}

pub fn verify_wire_index(
    document: &Value,
    pinned: Option<&PinnedSourceIdentity>,
    index_url: Option<&str>,
) -> Result<bool, FusionError> {
    let declared_key = public_key(document)?;
    if let Some(pin) = pinned {
        if source_id(document)? != pin.source_id
            || declared_key != pin.public_key.as_deref()
            || index_url.is_some_and(|url| url != pin.url)
        {
            return Err(invalid(
                "The source identity does not match its pinned public key and URL.",
            ));
        }
    }
    let signature = match document.get("signature") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.as_str()),
        _ => return Err(invalid("The source index signature is malformed.")),
    };
    let Some(key) = pinned
        .and_then(|pin| pin.public_key.as_deref())
        .or(declared_key)
    else {
        return if signature.is_some() {
            Err(invalid(
                "A signed source index must declare its public key.",
            ))
        } else {
            Ok(false)
        };
    };
    let signature =
        signature.ok_or_else(|| invalid("The keyed source index is missing its signature."))?;
    let key: [u8; 32] = STANDARD
        .decode(key)
        .map_err(|_| invalid("Invalid Ed25519 public key encoding."))?
        .try_into()
        .map_err(|_| invalid("An Ed25519 public key must contain 32 bytes."))?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| invalid("Invalid Ed25519 public key."))?;
    let signature = STANDARD
        .decode(signature)
        .map_err(|_| invalid("Invalid Ed25519 signature encoding."))?;
    let signature = Signature::from_slice(&signature)
        .map_err(|_| invalid("An Ed25519 signature must contain 64 bytes."))?;
    key.verify_strict(&signing_payload(document)?, &signature)
        .map_err(|_| {
            invalid("The source index signature does not match the original wire bytes.")
        })?;
    Ok(true)
}

pub fn sign_index(index: &mut SourceIndex, key: &SigningKey) -> Result<(), FusionError> {
    let public = STANDARD.encode(key.verifying_key().as_bytes());
    if index
        .source_info
        .public_key
        .as_ref()
        .is_some_and(|declared| declared != &public)
    {
        return Err(invalid(
            "The signing key differs from the source's public key.",
        ));
    }
    index.source_info.public_key = Some(public);
    index.signature = None;
    let document =
        serde_json::to_value(&index).map_err(|_| invalid("Cannot encode the source index."))?;
    index.signature = Some(STANDARD.encode(key.sign(&signing_payload(&document)?).to_bytes()));
    Ok(())
}

pub fn read_signing_key(path: &Path) -> Result<SigningKey, FusionError> {
    use std::io::Read;
    let path = absolute_key_path(path)?;
    let file = fs_safety::open_read(&path)?;
    let mut pem = String::new();
    file.take(PRIVATE_KEY_LIMIT + 1)
        .read_to_string(&mut pem)
        .map_err(|_| invalid("Cannot read the PKCS#8 signing key."))?;
    if pem.len() as u64 > PRIVATE_KEY_LIMIT {
        return Err(invalid("The PKCS#8 signing key exceeds its size limit."));
    }
    SigningKey::from_pkcs8_pem(&pem)
        .map_err(|_| invalid("Expected an Ed25519 PKCS#8 PEM signing key."))
}

#[derive(Debug, Serialize)]
pub struct KeygenReport {
    pub private_key_path: std::path::PathBuf,
    pub public_key: String,
}

fn absolute_key_path(path: &Path) -> Result<PathBuf, FusionError> {
    if path.as_os_str().is_empty()
        || (!path.is_absolute()
            && (path.has_root()
                || path
                    .components()
                    .any(|part| matches!(part, Component::Prefix(_)))))
    {
        return Err(FusionError::new(
            ErrorCode::InvalidPath,
            "Use an absolute or ordinary relative signing-key path.",
        ));
    }
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|error| fs_safety::io_error("Resolve key output", path, error))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(FusionError::new(
                        ErrorCode::InvalidPath,
                        "The signing-key path traverses above its filesystem root.",
                    ));
                }
            }
            part => normalized.push(part.as_os_str()),
        }
    }
    Ok(normalized)
}

pub fn keygen(path: &Path) -> Result<KeygenReport, FusionError> {
    let path = absolute_key_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("The key output has no parent directory."))?;
    let _guard = fs_safety::guard(parent)?;
    let key = SigningKey::generate(&mut OsRng);
    let pem = key
        .to_pkcs8_pem(Default::default())
        .map_err(|_| invalid("Cannot encode the signing key."))?;
    let mut file = create_private_key(&path)
        .map_err(|error| fs_safety::io_error("Create signing key", &path, error))?;
    file.write_all(pem.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| fs_safety::io_error("Write signing key", &path, error))?;
    Ok(KeygenReport {
        private_key_path: path,
        public_key: STANDARD.encode(key.verifying_key().as_bytes()),
    })
}

#[cfg(not(windows))]
fn create_private_key(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(windows)]
fn create_private_key(path: &Path) -> std::io::Result<std::fs::File> {
    windows_key_file::create(path)
}

#[cfg(windows)]
mod windows_key_file {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::path::Path;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit_handle: i32,
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *const SecurityAttributes,
            creation_disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    pub(super) fn create(path: &Path) -> std::io::Result<File> {
        let mut name: Vec<_> = path.as_os_str().encode_wide().collect();
        if name.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "The key path contains a NUL character.",
            ));
        }
        name.push(0);
        // Create the file with its final protected DACL. No inherited user or
        // group ACE can read the private material, even during creation.
        let descriptor_text: Vec<_> = "D:P(A;;FA;;;OW)(A;;FA;;;SY)\0".encode_utf16().collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: NUL-terminated UTF-16 input and writable descriptor output.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                descriptor_text.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let attributes = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor,
            inherit_handle: 0,
        };
        // SAFETY: The parent directory is held by the caller's no-reparse
        // guard. CREATE_NEW refuses an existing leaf; the descriptor outlives
        // creation. The resulting non-inheritable handle has exclusive access.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                0x4000_0000,
                0,
                &attributes,
                1,
                0x0020_0080,
                std::ptr::null_mut(),
            )
        };
        let error = (handle as isize == -1).then(std::io::Error::last_os_error);
        // SAFETY: Windows allocated this descriptor with LocalAlloc.
        unsafe {
            LocalFree(descriptor);
        }
        if let Some(error) = error {
            return Err(error);
        }
        // SAFETY: This newly created valid handle is transferred exactly once.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    #[cfg(test)]
    pub(super) fn descriptor(file: &File) -> String {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn GetSecurityInfo(
                handle: *mut c_void,
                kind: u32,
                information: u32,
                owner: *mut *mut c_void,
                group: *mut *mut c_void,
                dacl: *mut *mut c_void,
                sacl: *mut *mut c_void,
                descriptor: *mut *mut c_void,
            ) -> u32;
            fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor: *const c_void,
                revision: u32,
                information: u32,
                text: *mut *mut u16,
                length: *mut u32,
            ) -> i32;
        }
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: file owns a readable handle, other security outputs unused.
        let result = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                1,
                4,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(result, 0);
        let mut text = std::ptr::null_mut();
        let mut length = 0;
        // SAFETY: descriptor came from GetSecurityInfo, writable outputs.
        let result = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                1,
                4,
                &mut text,
                &mut length,
            )
        };
        assert_ne!(result, 0);
        // SAFETY: API returned length UTF-16 units including the terminator.
        let value = unsafe {
            String::from_utf16_lossy(std::slice::from_raw_parts(
                text,
                length.saturating_sub(1) as usize,
            ))
        };
        // SAFETY: Both returned allocations use LocalAlloc.
        unsafe {
            LocalFree(text.cast());
            LocalFree(descriptor);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn signed() -> Value {
        let key = SigningKey::from_bytes(&[7; 32]);
        let mut index: SourceIndex = serde_json::from_value(json!({"schema_version":"1.0","source_info":{"id":"signed","name":"Signed"},"games":[{"id":"g","title":"G","version":"1","manifest_url":"relative.json","manifest_blake3":"A".repeat(64)}]})).unwrap();
        sign_index(&mut index, &key).unwrap();
        serde_json::to_value(index).unwrap()
    }

    #[test]
    fn verifies_original_wire_and_rejects_tampering_and_downgrades() {
        let document = signed();
        assert!(verify_wire_index(&document, None, None).unwrap());
        let pin = PinnedSourceIdentity {
            source_id: "signed".into(),
            url: "https://example.org/index.json".into(),
            public_key: public_key(&document).unwrap().map(str::to_owned),
        };
        assert!(verify_wire_index(&document, Some(&pin), Some(&pin.url)).unwrap());
        for field in ["manifest_blake3", "manifest_url"] {
            let mut changed = document.clone();
            changed["games"][0][field] = json!("changed");
            assert_eq!(
                verify_wire_index(&changed, Some(&pin), None)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidSourceSignature
            );
        }
        for field in ["signature", "public_key"] {
            let mut changed = document.clone();
            if field == "signature" {
                changed.as_object_mut().unwrap().remove(field);
            } else {
                changed["source_info"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
            }
            assert!(verify_wire_index(&changed, Some(&pin), None).is_err());
        }
        let mut changed = document;
        changed["source_info"]["public_key"] =
            json!(STANDARD.encode(SigningKey::from_bytes(&[8; 32]).verifying_key().as_bytes()));
        assert!(verify_wire_index(&changed, Some(&pin), None).is_err());
    }

    #[test]
    fn canonical_keys_whitespace_duplicates_and_unsigned_compatibility() {
        let document = signed();
        let pretty = serde_json::to_string_pretty(&document).unwrap();
        assert!(verify_wire_index(&parse_wire_index(&pretty).unwrap(), None, None).unwrap());
        let expected = b"FusionSourceIndex.v1\0{\"a\":{\"a\":2,\"z\":1},\"z\":[3,2]}";
        assert_eq!(
            signing_payload(&json!({"z":[3,2],"a":{"z":1,"a":2},"signature":"ignored"})).unwrap(),
            expected
        );
        for text in [
            r#"{"source_info":{"id":"a","id":"b"}}"#,
            r#"{"games":[{"id":"g","id":"h"}]}"#,
        ] {
            assert!(parse_wire_index(text).is_err());
        }
        assert!(
            !verify_wire_index(&json!({"source_info":{"id":"a"},"games":[]}), None, None).unwrap()
        );
    }

    #[test]
    fn keygen_round_trip_does_not_overwrite_or_disclose_private_material() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private.pem");
        let report = keygen(&path).unwrap();
        let key = read_signing_key(&path).unwrap();
        assert_eq!(
            report.public_key,
            STANDARD.encode(key.verifying_key().as_bytes())
        );
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("PRIVATE KEY"));
        let original = std::fs::read(&path).unwrap();
        assert!(keygen(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let nested = temp.path().join("subdirectory");
        std::fs::create_dir(&nested).unwrap();
        let normalized = nested.join(".").join("..").join("normalized.pem");
        assert_eq!(
            keygen(&normalized).unwrap().private_key_path,
            temp.path().join("normalized.pem")
        );
        #[cfg(windows)]
        {
            let file = fs_safety::open_read(&path).unwrap();
            let descriptor = windows_key_file::descriptor(&file);
            assert!(descriptor.starts_with("D:P"), "{descriptor}");
            assert!(descriptor.contains(";;;OW)"), "{descriptor}");
            assert!(descriptor.contains(";;;SY)"), "{descriptor}");
            assert!(!descriptor.contains(";;;WD)"), "{descriptor}");
            assert!(!descriptor.contains(";;;BU)"), "{descriptor}");
        }
    }
}
