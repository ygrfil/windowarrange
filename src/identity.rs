pub const APP_ID: &str = "table-arranger-control";
pub const PANEL_TITLE: &str = "Table Arranger Control";
pub const PRODUCT_NAME: &str = "Table Arranger Control";

#[cfg(test)]
mod tests {
    use super::{APP_ID, PANEL_TITLE, PRODUCT_NAME};

    #[test]
    fn outer_process_identity_does_not_look_like_a_poker_client() {
        for value in [APP_ID, PANEL_TITLE, PRODUCT_NAME] {
            assert!(!value.to_ascii_lowercase().contains("clubgg"));
        }
    }
}
