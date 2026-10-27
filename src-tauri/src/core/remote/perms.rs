//! Collaborators' permissions (protocol 2 §2): the 10 codes the owner picks per collaborator.
//! The app checks them again on every command; unknown codes are ignored (later additions).
//! Toward a protocol 1 server the actor has a role instead, mapped to the same set.

/// Take and clear layers.
pub const TAKE_CLEAR: &str = "take_clear";
/// Edit fields of layers that aren't on air.
pub const EDIT_FIELDS: &str = "edit_fields";
/// Edit fields of layers on air (always given with `edit_fields`).
pub const EDIT_LIVE_FIELDS: &str = "edit_live_fields";
pub const RUN_CUES: &str = "run_cues";
pub const STUDIO: &str = "studio";
pub const TIMERS: &str = "timers";
pub const VARIANTS: &str = "variants";
/// Select or step a data source's current row.
pub const ROWS: &str = "rows";
pub const BUTTON_PANEL: &str = "button_panel";
/// Watch the live picture (server-side only; never in a command).
pub const WATCH_PREVIEW: &str = "watch_preview";

/// Every code, in the order the invite form lists them.
pub const ALL: [&str; 10] = [
    TAKE_CLEAR,
    EDIT_FIELDS,
    EDIT_LIVE_FIELDS,
    RUN_CUES,
    STUDIO,
    TIMERS,
    VARIANTS,
    ROWS,
    BUTTON_PANEL,
    WATCH_PREVIEW,
];

/// A set of known permissions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Perms(u16);

impl Perms {
    pub const ALL: Perms = Perms((1 << ALL.len()) - 1);

    fn bit(code: &str) -> Option<u16> {
        ALL.iter().position(|c| *c == code).map(|i| 1 << i)
    }

    /// From codes; unknown ones are ignored. `edit_live_fields` brings `edit_fields` (the server
    /// adds it too).
    pub fn from_codes<S: AsRef<str>>(codes: &[S]) -> Perms {
        let mut p = Perms(0);
        for c in codes {
            if let Some(b) = Self::bit(c.as_ref()) {
                p.0 |= b;
            }
        }
        if p.has(EDIT_LIVE_FIELDS) {
            p.0 |= Self::bit(EDIT_FIELDS).unwrap_or(0);
        }
        p
    }

    /// A protocol 1 role (§8 of protocol 2: the server puts the role that allows exactly the
    /// command it checked). `owner` / `operator`: what protocol 1 operators could do.
    pub fn from_role(role: &str) -> Perms {
        match role {
            "owner" => Perms::ALL,
            "operator" => Perms::from_codes(&[TAKE_CLEAR, EDIT_LIVE_FIELDS, RUN_CUES]),
            "data_editor" => Perms::from_codes(&[EDIT_FIELDS]),
            _ => Perms(0),
        }
    }

    pub fn has(self, code: &str) -> bool {
        Self::bit(code).is_some_and(|b| self.0 & b != 0)
    }

    /// The codes, in [`ALL`]'s order.
    pub fn codes(self) -> Vec<String> {
        ALL.iter()
            .filter(|c| self.has(c))
            .map(|c| c.to_string())
            .collect()
    }
}

/// The codes the app sends to the collaborators API: known ones only, in order, no duplicates,
/// `edit_fields` with `edit_live_fields`.
pub fn normalize<S: AsRef<str>>(codes: &[S]) -> Vec<String> {
    Perms::from_codes(codes).codes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_unknown_ones_are_ignored() {
        let p = Perms::from_codes(&["rows", "future_thing", "take_clear", "rows"]);
        assert_eq!(p.codes(), vec!["take_clear", "rows"]);
        assert!(!p.has("future_thing"));
        assert_eq!(Perms::ALL.codes().len(), 10);
        assert_eq!(
            normalize(&["edit_live_fields"]),
            vec!["edit_fields", "edit_live_fields"]
        );
    }

    #[test]
    fn protocol_1_roles_map_to_what_they_allowed() {
        let op = Perms::from_role("operator");
        assert!(op.has(TAKE_CLEAR) && op.has(EDIT_FIELDS) && op.has(EDIT_LIVE_FIELDS));
        assert!(op.has(RUN_CUES) && !op.has(STUDIO));
        let de = Perms::from_role("data_editor");
        assert_eq!(de.codes(), vec![EDIT_FIELDS]);
        assert_eq!(Perms::from_role("viewer"), Perms::default());
        assert_eq!(Perms::from_role("owner"), Perms::ALL);
    }
}
