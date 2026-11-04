use super::decode;

#[test]
fn decodes_named_entities() {
    assert_eq!(decode("Tom &amp; Jerry"), "Tom & Jerry");
    assert_eq!(decode("&laquo;Мозг&raquo;"), "«Мозг»");
}

#[test]
fn decodes_decimal_and_hex_entities() {
    assert_eq!(decode("We&#39;re"), "We're");
    assert_eq!(decode("&#1053;&#x435;&#x439;"), "Ней");
}

#[test]
fn leaves_an_ampersand_that_is_not_an_entity_alone() {
    assert_eq!(decode("R&D and & more"), "R&D and & more");
    assert_eq!(decode("a &notanentity; b"), "a &notanentity; b");
}

#[test]
fn a_run_of_entities_decodes_without_losing_the_text_between_them() {
    assert_eq!(decode("&lt;a&gt; и &lt;b&gt;"), "<a> и <b>");
}
