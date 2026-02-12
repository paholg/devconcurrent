use serde::de::{self, Unexpected};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortMap {
    pub host: u16,
    pub container: u16,
}

impl Serialize for PortMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{}:{}", self.host, self.container))
    }
}

impl<'de> Deserialize<'de> for PortMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Number(u16),
            String(String),
        }

        match Raw::deserialize(deserializer)? {
            Raw::Number(port) => Ok(PortMap {
                host: port,
                container: port,
            }),
            Raw::String(s) => {
                if let Some((host, container)) = s.split_once(':') {
                    let host = host.parse::<u16>().map_err(|_| {
                        de::Error::invalid_value(Unexpected::Str(&s), &"a valid port mapping")
                    })?;
                    let container = container.parse::<u16>().map_err(|_| {
                        de::Error::invalid_value(Unexpected::Str(&s), &"a valid port mapping")
                    })?;
                    Ok(PortMap {
                        host: host,
                        container,
                    })
                } else {
                    let port = s.parse::<u16>().map_err(|_| {
                        de::Error::invalid_value(
                            Unexpected::Str(&s),
                            &"a port number or \"host:container\" mapping",
                        )
                    })?;
                    Ok(PortMap {
                        host: port,
                        container: port,
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_number() {
        let pm: PortMap = serde_json::from_str("3000").unwrap();
        assert_eq!(
            pm,
            PortMap {
                host: 3000,
                container: 3000
            }
        );
    }

    #[test]
    fn from_string_plain() {
        let pm: PortMap = serde_json::from_str("\"3000\"").unwrap();
        assert_eq!(
            pm,
            PortMap {
                host: 3000,
                container: 3000
            }
        );
    }

    #[test]
    fn from_string_mapping() {
        let pm: PortMap = serde_json::from_str("\"3000:3001\"").unwrap();
        assert_eq!(
            pm,
            PortMap {
                host: 3000,
                container: 3001
            }
        );
    }

    #[test]
    fn serialize_mapping() {
        let pm = PortMap {
            host: 3000,
            container: 3001,
        };
        assert_eq!(serde_json::to_string(&pm).unwrap(), "\"3000:3001\"");
    }

    #[test]
    fn invalid_string() {
        assert!(serde_json::from_str::<PortMap>("\"abc\"").is_err());
    }

    #[test]
    fn invalid_mapping() {
        assert!(serde_json::from_str::<PortMap>("\"abc:3000\"").is_err());
        assert!(serde_json::from_str::<PortMap>("\"3000:abc\"").is_err());
    }
}
