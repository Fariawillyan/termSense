//! Java class file versions: `class file version 65.0` → Java 21.
//!
//! `UnsupportedClassVersionError` reports the version a class was compiled
//! for and the newest one the running JVM accepts, as class file numbers.
//! Since Java 5 (49), each Java release adds one: Java N = N + 44.

/// Java release that introduced class file major version `major`.
pub fn java_version(major: u16) -> Option<String> {
    Some(match major {
        45 => "1.1".to_string(),
        46 => "1.2".to_string(),
        47 => "1.3".to_string(),
        48 => "1.4".to_string(),
        49..=200 => (major - 44).to_string(),
        _ => return None,
    })
}

/// Class file versions mentioned in `text`, in order and without repeats:
/// `class file version 65.0`, `class file versions up to 61.0`,
/// `Unsupported major.minor version 52.0`, `class file 61`.
pub fn class_versions(text: &str) -> Vec<u16> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    for key in ["class file", "major.minor version", "major version"] {
        let mut rest = lower.as_str();
        while let Some(i) = rest.find(key) {
            rest = &rest[i + key.len()..];
            // Skip the words between the key and the number ("versions up to").
            let window: String = rest.chars().take(24).collect();
            let digits: String = window
                .trim_start_matches(|c: char| !c.is_ascii_digit())
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(v) = digits.parse::<u16>()
                && java_version(v).is_some()
                && !out.contains(&v)
            {
                out.push(v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(java_version(52).as_deref(), Some("8"));
        assert_eq!(java_version(61).as_deref(), Some("17"));
        assert_eq!(java_version(65).as_deref(), Some("21"));
        assert_eq!(java_version(69).as_deref(), Some("25"));
        assert_eq!(java_version(48).as_deref(), Some("1.4"));
        assert_eq!(java_version(30), None);
    }

    #[test]
    fn finds_versions_in_error_messages() {
        let msg = "java.lang.UnsupportedClassVersionError: com/x/App has been compiled by a more recent version of the Java Runtime (class file version 65.0), this version of the Java Runtime only recognizes class file versions up to 61.0";
        assert_eq!(class_versions(msg), [65, 61]);
        assert_eq!(class_versions("Unsupported major.minor version 52.0"), [52]);
        assert_eq!(class_versions("class file 61"), [61]);
        assert!(class_versions("versão do arquivo 61").is_empty());
    }
}
