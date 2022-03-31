use super::{file::*, *};

#[cfg(feature = "ron_config")]
/// RON file config
#[derive(Debug)]
pub struct RONConfig;
#[cfg(feature = "ron_config")]
impl ConfigFileType for RONConfig {
    fn extension() -> &'static str {
        "ron"
    }

    fn serialize<C: Serialize, W: Write>(config: &C, writer: &mut W) -> anyhow::Result<()> {
        ron::ser::to_writer_pretty(writer, config, ron::ser::PrettyConfig::default())
            .map_err(|e| anyhow!(e))
    }

    fn deserialize<C: DeserializeOwned, R: Read>(reader: &mut R) -> anyhow::Result<C> {
        ron::de::from_reader(reader).map_err(|e| anyhow!(e))
    }
}
#[cfg(feature = "ron_config")]
impl ValueType for RONConfig {
    type Value = ron::Value;
}

#[cfg(feature = "json_config")]
/// JSON file config
#[derive(Debug)]
pub struct JSONConfig;
#[cfg(feature = "json_config")]
impl ConfigFileType for JSONConfig {
    fn extension() -> &'static str {
        "json"
    }

    fn serialize<C: Serialize, W: Write>(config: &C, writer: &mut W) -> anyhow::Result<()> {
        serde_json::to_writer_pretty(writer, config).map_err(|e| anyhow!(e))
    }

    fn deserialize<C: DeserializeOwned, R: Read>(reader: &mut R) -> anyhow::Result<C> {
        serde_json::from_reader(reader).map_err(|e| anyhow!(e))
    }
}
#[cfg(feature = "json_config")]
impl ValueType for JSONConfig {
    type Value = serde_json::Value;
}

#[cfg(feature = "toml_config")]
/// TOML file config
#[derive(Debug)]
pub struct TOMLConfig;
#[cfg(feature = "toml_config")]
impl ConfigFileType for TOMLConfig {
    fn extension() -> &'static str {
        "toml"
    }

    fn serialize<C: Serialize, W: Write>(config: &C, writer: &mut W) -> anyhow::Result<()> {
        let s = toml::to_string_pretty(config).map_err(|e| anyhow!(e))?;
        writer.write_all(s.as_bytes()).map_err(|e| anyhow!(e))
    }

    fn deserialize<C: DeserializeOwned, R: Read>(reader: &mut R) -> anyhow::Result<C> {
        let mut s = String::new();
        reader.read_to_string(&mut s).map_err(|e| anyhow!(e))?;
        toml::from_str(&s).map_err(|e| anyhow!(e))
    }
}
#[cfg(feature = "toml_config")]
impl ValueType for TOMLConfig {
    type Value = toml::Value;
}
