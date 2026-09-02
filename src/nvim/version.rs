use rmpv::Value;
use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct NvimVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub api_level: u64,
    pub api_compatible: u64,
    pub api_prerelease: bool,
}

impl NvimVersion {
    pub fn supports_api(self, api_level: u64) -> bool {
        self.api_level >= api_level
    }
}

impl fmt::Display for NvimVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.major == 0 && self.minor == 0 && self.patch == 0 {
            return write!(formatter, "API {}", self.api_level);
        }

        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NvimCapabilities {
    pub ui_options: BTreeSet<String>,
    pub ui_events: BTreeSet<String>,
}

impl NvimCapabilities {
    pub fn supports_ui_option(&self, option: &str) -> bool {
        self.ui_options.is_empty() || self.ui_options.contains(option)
    }

    pub fn supports_ui_event(&self, event: &str) -> bool {
        self.ui_events.is_empty() || self.ui_events.contains(event)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NvimProtocolInfo {
    pub version: NvimVersion,
    pub capabilities: NvimCapabilities,
}

pub(super) fn parse_protocol_info(api_info: &Value) -> Result<NvimProtocolInfo, String> {
    let metadata = api_info
        .as_array()
        .and_then(|values| values.get(1))
        .ok_or_else(|| "nvim_get_api_info response has no metadata".to_owned())?;
    let version_value = map_value(metadata, "version")
        .ok_or_else(|| "nvim_get_api_info response has no version".to_owned())?;
    let api_level = map_value(version_value, "api_level")
        .and_then(Value::as_u64)
        .ok_or_else(|| "nvim_get_api_info response has no api level".to_owned())?;

    let version = NvimVersion {
        major: map_value(version_value, "major")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        minor: map_value(version_value, "minor")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        patch: map_value(version_value, "patch")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        api_level,
        api_compatible: map_value(version_value, "api_compatible")
            .and_then(Value::as_u64)
            .unwrap_or(api_level),
        api_prerelease: map_value(version_value, "api_prerelease")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };

    let capabilities = NvimCapabilities {
        ui_options: metadata_names(map_value(metadata, "ui_options")),
        ui_events: metadata_names(map_value(metadata, "ui_events")),
    };

    Ok(NvimProtocolInfo {
        version,
        capabilities,
    })
}

fn metadata_names(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            string_value(entry).or_else(|| map_value(entry, "name").and_then(string_value))
        })
        .collect()
}

fn map_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Map(entries) = value else {
        return None;
    };

    entries.iter().find_map(|(entry_key, entry_value)| {
        (string_value(entry_key).as_deref() == Some(key)).then_some(entry_value)
    })
}

fn string_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => value.as_str().map(str::to_owned),
        _ => None,
    }
}
