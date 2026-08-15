//! Parse advertised UCI options and map GUI resource knobs onto the names
//! each engine actually implements.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvertisedUciOption {
    pub name: String,
    pub kind: UciOptionKind,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UciOptionKind {
    Check,
    Spin,
    Combo,
    Button,
    String,
}

const HASH_ALIASES: &[&str] = &["Hash", "HashSize", "TTSize", "Hash Table Size"];
const THREAD_ALIASES: &[&str] = &["Threads", "NumThreads", "Cores"];
const BOOK_ALIASES: &[&str] = &["OwnBook", "OwnBookUsage"];

pub fn parse_uci_option_line(line: &str) -> Option<AdvertisedUciOption> {
    let rest = line.strip_prefix("option name ")?;
    let (name, after_name) = rest.split_once(" type ")?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut tokens = after_name.split_whitespace();
    let kind = match tokens.next()? {
        "check" => UciOptionKind::Check,
        "spin" => UciOptionKind::Spin,
        "combo" => UciOptionKind::Combo,
        "button" => UciOptionKind::Button,
        "string" => UciOptionKind::String,
        _ => return None,
    };
    let mut min = None;
    let mut max = None;
    while let Some(token) = tokens.next() {
        match token {
            "min" => min = tokens.next().and_then(|value| value.parse().ok()),
            "max" => max = tokens.next().and_then(|value| value.parse().ok()),
            _ => {}
        }
    }
    Some(AdvertisedUciOption {
        name: name.to_owned(),
        kind,
        min,
        max,
    })
}

pub fn has_option(advertised: &[AdvertisedUciOption], name: &str) -> bool {
    advertised
        .iter()
        .any(|option| option.name.eq_ignore_ascii_case(name))
}

pub fn resolve_option_name<'a>(
    advertised: &'a [AdvertisedUciOption],
    aliases: &[&str],
) -> Option<&'a str> {
    aliases.iter().find_map(|alias| {
        advertised
            .iter()
            .find(|option| option.name.eq_ignore_ascii_case(alias))
            .map(|option| option.name.as_str())
    })
}

pub fn clamp_spin(advertised: &[AdvertisedUciOption], name: &str, value: i64) -> i64 {
    let Some(option) = advertised
        .iter()
        .find(|option| option.name.eq_ignore_ascii_case(name))
    else {
        return value;
    };
    let mut next = value;
    if let Some(min) = option.min {
        next = next.max(min);
    }
    if let Some(max) = option.max {
        next = next.min(max);
    }
    next
}

pub fn hash_option_name(advertised: &[AdvertisedUciOption]) -> Option<&str> {
    resolve_option_name(advertised, HASH_ALIASES)
}

pub fn threads_option_name(advertised: &[AdvertisedUciOption]) -> Option<&str> {
    resolve_option_name(advertised, THREAD_ALIASES)
}

pub fn own_book_option_name(advertised: &[AdvertisedUciOption]) -> Option<&str> {
    resolve_option_name(advertised, BOOK_ALIASES)
}

/// When the engine advertised no options, keep the historical Hash/Threads names.
pub fn routed_setoption_commands(
    advertised: &[AdvertisedUciOption],
    hash_mb: Option<usize>,
    threads: Option<usize>,
    own_book: Option<bool>,
    custom: &[(String, String)],
) -> Vec<(String, String)> {
    let strict = !advertised.is_empty();
    let mut commands = Vec::new();
    if let Some(hash_mb) = hash_mb {
        let name = if strict {
            hash_option_name(advertised)
        } else {
            Some("Hash")
        };
        if let Some(name) = name {
            let value = clamp_spin(advertised, name, hash_mb as i64);
            commands.push((name.to_owned(), value.to_string()));
        }
    }
    if let Some(threads) = threads {
        let name = if strict {
            threads_option_name(advertised)
        } else {
            Some("Threads")
        };
        if let Some(name) = name {
            let value = clamp_spin(advertised, name, threads as i64);
            commands.push((name.to_owned(), value.to_string()));
        }
    }
    if let Some(own_book) = own_book {
        let name = if strict {
            own_book_option_name(advertised)
        } else {
            Some("OwnBook")
        };
        if let Some(name) = name {
            commands.push((name.to_owned(), own_book.to_string()));
        }
    }
    for (name, value) in custom {
        if !strict || has_option(advertised, name) {
            commands.push((name.clone(), value.clone()));
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stockfish_hash_and_lc0_weights() {
        let hash =
            parse_uci_option_line("option name Hash type spin default 16 min 1 max 33554432")
                .expect("hash");
        assert_eq!(hash.name, "Hash");
        assert_eq!(hash.kind, UciOptionKind::Spin);
        assert_eq!(hash.min, Some(1));
        assert_eq!(hash.max, Some(33_554_432));
        let weights =
            parse_uci_option_line("option name WeightsFile type string default <autodiscover>")
                .expect("weights");
        assert_eq!(weights.name, "WeightsFile");
        assert_eq!(weights.kind, UciOptionKind::String);
        assert!(parse_uci_option_line("uciok").is_none());
    }

    #[test]
    fn lc0_does_not_receive_hash_when_it_never_advertised_it() {
        let advertised = vec![
            parse_uci_option_line("option name Threads type spin default 2 min 1 max 128").unwrap(),
            parse_uci_option_line("option name WeightsFile type string default").unwrap(),
        ];
        let commands = routed_setoption_commands(
            &advertised,
            Some(128),
            Some(4),
            Some(false),
            &[
                ("UseNNUE".to_owned(), "true".to_owned()),
                ("WeightsFile".to_owned(), "/nets/bt4.pb.gz".to_owned()),
            ],
        );
        assert!(
            commands
                .iter()
                .all(|(name, _)| !name.eq_ignore_ascii_case("Hash"))
        );
        assert_eq!(
            commands
                .iter()
                .find(|(name, _)| name == "Threads")
                .map(|(_, value)| value.as_str()),
            Some("4")
        );
        assert_eq!(
            commands
                .iter()
                .find(|(name, _)| name == "WeightsFile")
                .map(|(_, value)| value.as_str()),
            Some("/nets/bt4.pb.gz")
        );
        assert!(commands.iter().all(|(name, _)| name != "UseNNUE"));
        assert!(commands.iter().all(|(name, _)| name != "OwnBook"));
    }

    #[test]
    fn hash_alias_and_spin_clamp() {
        let advertised = vec![
            parse_uci_option_line("option name HashSize type spin default 16 min 1 max 64")
                .unwrap(),
        ];
        let commands = routed_setoption_commands(&advertised, Some(512), None, None, &[]);
        assert_eq!(commands, vec![("HashSize".to_owned(), "64".to_owned())]);
    }

    #[test]
    fn empty_advertisement_keeps_legacy_hash_threads() {
        let commands = routed_setoption_commands(&[], Some(32), Some(2), Some(false), &[]);
        assert_eq!(
            commands,
            vec![
                ("Hash".to_owned(), "32".to_owned()),
                ("Threads".to_owned(), "2".to_owned()),
                ("OwnBook".to_owned(), "false".to_owned()),
            ]
        );
    }
}
