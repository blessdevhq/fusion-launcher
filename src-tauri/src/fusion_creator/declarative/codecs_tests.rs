use super::*;
use crate::fusion_source::program_schema::ProgramDescriptor;

#[test]
fn ini_rejects_addressed_ambiguity_but_keeps_unmanaged_duplicates() {
    for input in [
        "[Video]\nenabled\\default=false\nenabled\\default=true\n",
        "[Video]\nx=1\n[Video]\nenabled\\default=false\n",
    ] {
        assert!(render_config(&ini(), Some(input), &[setting("enabled", json!(true))]).is_err());
    }
    let original = "[Private]\ntoken=one\ntoken=two\n[Private]\nx=3\n";
    let output = render_config(&ini(), Some(original), &[setting("enabled", json!(true))]).unwrap();
    assert!(output.starts_with(original));
}

#[test]
fn ini_rust_callers_cannot_inject_addresses_or_mismatched_codecs() {
    for key in ["bad\nkey", "x=y", " key "] {
        let mut config = ini();
        if let ProgramConfigAddress::Ini { key: name, .. } = &mut config.keys[0].address {
            *name = key.into();
        }
        assert!(render_config(&config, None, &[setting("enabled", json!(true))]).is_err());
    }
    let mut config = ini();
    config.preserve_unmanaged = false;
    assert!(render_config(&config, None, &[setting("enabled", json!(true))]).is_err());
    config.preserve_unmanaged = true;
    config.format = ProgramConfigFormat::XmlAttributes;
    assert!(render_config(&config, None, &[setting("enabled", json!(true))]).is_err());
}
use serde_json::json;

fn frozen_stk() -> ProgramDescriptor {
    serde_json::from_str(include_str!("../../../../fixtures/creator/programs/supertuxkart-1.5.json")).unwrap()
}

#[test]
fn xml_frozen_stk_seed_then_profile_change_preserves_private_state() {
    let descriptor = frozen_stk();
    let config = &descriptor.configs[0];
    let windowed = &descriptor.build_profiles[0].settings;
    let fullscreen = &descriptor.build_profiles[1].settings;
    let seed = render_config(config, None, windowed).unwrap();
    assert!(seed.contains("<stkconfig version=\"8\""));
    assert!(seed.contains("fullscreen=\"false\""));
    assert!(seed.contains("show_fps=\"false\""));
    assert!(seed.contains("music_on=\"true\""));
    let original = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- private -->\n<stkconfig version=\"9\" token=\"a&amp;b&quot;c\">\n  <Video fullscreen='false' show_fps=\"false\" real_width=\"1920\"/>\n  <Audio music_on=\"false\" private=\"keep\"/>\n  <Players><Player name=\"Alice\"/><Player name=\"Bob\"/></Players>\n</stkconfig>\n";
    let changed = render_config(config, Some(original), fullscreen).unwrap();
    let expected = original.replace("fullscreen='false'", "fullscreen='true'")
        .replace("show_fps=\"false\"", "show_fps=\"true\"")
        .replace("music_on=\"false\"", "music_on=\"true\"");
    assert_eq!(changed, expected);
    assert_eq!(render_config(config, Some(&changed), fullscreen).unwrap(), changed);
    let back = render_config(config, Some(&changed), windowed).unwrap();
    assert!(back.contains("fullscreen='false'"));
    assert!(back.contains("version=\"9\""));
    assert!(back.contains("<Player name=\"Bob\"/>"));
}

#[test]
fn xml_creates_direct_child_path_and_escapes_only_scalar_values() {
    let mut config = frozen_stk().configs.remove(0);
    config.id = "main".into();
    config.keys.truncate(1);
    config.keys[0].id = "enabled".into();
    config.keys[0].address = ProgramConfigAddress::XmlAttribute {
        elements: vec!["Video".into(), "Display".into()], attribute: "label".into(),
    };
    config.keys[0].scalar_type = ProgramScalarType::String;
    let value = "A & B < C > D \" '";
    config.keys[0].allowed_values = vec![ProgramScalar::String(value.into())];
    let output = render_config(&config, Some("<stkconfig><Video/><!--keep--></stkconfig>"), &[setting("enabled", json!(value))]).unwrap();
    assert!(output.contains("<Video><Display label=\"A &amp; B &lt; C &gt; D &quot; &apos;\"/></Video>"));
    assert!(output.contains("<!--keep-->"));
    assert_eq!(render_config(&config, Some(&output), &[setting("enabled", json!(value))]).unwrap(), output);
}

#[test]
fn xml_rejects_malformed_unsafe_or_ambiguous_existing_state() {
    let descriptor = frozen_stk();
    for input in [
        "", "<other/>", "<stkconfig/><stkconfig/>",
        "<!DOCTYPE stkconfig [<!ENTITY x 'secret'>]><stkconfig/>",
        "<stkconfig xmlns='urn:test'/>", "<stkconfig><x:include/></stkconfig>",
        "<?unsafe x?><stkconfig/>", "<stkconfig><?unsafe x?></stkconfig>",
        "<stkconfig><![CDATA[secret]]></stkconfig>",
        "<stkconfig x='&custom;'/>", "<stkconfig x='&'/>", "<stkconfig>&custom;</stkconfig>",
        "<stkconfig><Video/><Video/></stkconfig>",
        "<stkconfig><Video x='1' x='2'/></stkconfig>",
        "<stkconfig><Video></Audio></stkconfig>", "<stkconfig><Video></stkconfig>",
        "<stkconfig x=unquoted/>", "<stkconfig x='a<b'/>",
        "<stkconfig x='1'y='2'/>", "<stkconfig / >", "<stkconfig></ stkconfig>",
        "<stkconfig><!-- bad -- comment --></stkconfig>",
        "<stkconfig>]]></stkconfig>", "<stkconfig>\u{0}</stkconfig>",
        "<stkconfig x='\u{1}'/>", "<stkconfig><1bad/></stkconfig>",
        "text<stkconfig/>", "<stkconfig/>text", "<stkconfig>",
        "<!-- hi --><?xml version='1.0'?><stkconfig/>",
        "<?xml version='1.1'?><stkconfig/>",
    ] {
        assert!(render_config(&descriptor.configs[0], Some(input), &descriptor.build_profiles[0].settings).is_err(), "accepted {input:?}");
    }
}

#[test]
fn xml_rejects_resource_exhaustion_and_rust_constructed_invalid_seeds() {
    let descriptor = frozen_stk();
    let config = &descriptor.configs[0];
    let settings = &descriptor.build_profiles[0].settings;
    for input in [
        format!("<stkconfig>{}</stkconfig>", " ".repeat(2 * 1024 * 1024)),
        format!("<stkconfig>{}{}</stkconfig>", "<n>".repeat(33), "</n>".repeat(33)),
        format!("<stkconfig>{}</stkconfig>", "<n/>".repeat(16_385)),
        format!("<stkconfig {}/>", (0..129).map(|n| format!("a{n}='v' ")).collect::<String>()),
    ] {
        assert!(render_config(config, Some(&input), settings).is_err());
    }
    let mut invalid = config.clone();
    if let ProgramConfigSeed::XmlRoot { root, .. } = &mut invalid.seed { *root = "bad:name".into(); }
    assert!(render_config(&invalid, None, settings).is_err());
    let mut invalid = config.clone();
    if let ProgramConfigSeed::XmlRoot { attributes, .. } = &mut invalid.seed { attributes.push(attributes[0].clone()); }
    assert!(render_config(&invalid, None, settings).is_err());
}

#[test]
fn settings_require_exactly_one_typed_allowed_value_per_declared_key() {
    let config = ini();
    let valid = setting("enabled", json!(true));
    for settings in [
        vec![],
        vec![valid.clone(), valid.clone()],
        vec![valid.clone(), setting("unknown", json!(true))],
        vec![setting("enabled", json!("true"))],
    ] {
        assert!(render_config(&config, None, &settings).is_err(), "{settings:?}");
    }
    let mut limited = config.clone();
    limited.keys[0].allowed_values = vec![scalar(json!(false))];
    assert!(render_config(&limited, None, &[valid.clone()]).is_err());
    let mut unrelated = valid.clone();
    unrelated.config_id = "other-config".into();
    assert!(render_config(&config, None, &[unrelated, valid]).is_ok());
    let mut invalid_domain = config;
    invalid_domain.keys[0].allowed_values.push(scalar(json!(7)));
    assert!(render_config(&invalid_domain, None, &[setting("enabled", json!(true))]).is_err());
}

fn scalar(value: serde_json::Value) -> ProgramScalar {
    serde_json::from_value(value).unwrap()
}

#[test]
fn ini_serializes_typed_scalars_as_safe_literals() {
    for (kind, value, expected) in [
        (ProgramScalarType::Integer, json!(12.0), "12"),
        (ProgramScalarType::Number, json!(0.125), "0.125"),
        (ProgramScalarType::Number, json!(0.0000001), "0.0000001"),
        (ProgramScalarType::String, json!(" a\";#=\\b "), "\" a\\\";#=\\\\b \""),
    ] {
        let mut config = ini();
        config.keys[0].scalar_type = kind;
        config.keys[0].allowed_values = vec![scalar(value.clone())];
        let output = render_config(&config, None, &[setting("enabled", value)]).unwrap();
        assert_eq!(output, format!("[Video]\nenabled\\default={expected}\n"));
    }
}

fn ini() -> ProgramConfig {
    serde_json::from_value(json!({
        "id":"main", "path":"private/settings.ini", "format":"ini",
        "seed":{"kind":"empty_ini"}, "preserve_unmanaged":true,
        "keys":[{"id":"enabled", "address":{"kind":"ini","section":"Video","key":"enabled\\default"},
            "scalar_type":"boolean", "allowed_values":[false,true]}]
    })).unwrap()
}

fn setting(key: &str, value: serde_json::Value) -> ProgramSetting {
    ProgramSetting { config_id: "main".into(), key_id: key.into(), value: scalar(value) }
}

#[test]
fn ini_seed_renders_boolean_with_literal_backslash_key() {
    let output = render_config(&ini(), None, &[setting("enabled", json!(true))]).unwrap();
    assert_eq!(output, "[Video]\nenabled\\default=true\n");
}

#[test]
fn ini_updates_only_managed_key_preserving_private_lines_and_crlf() {
    let original = "; private comment\r\n[Video]\r\ntoken=secret\r\nenabled\\default=false\r\n[Private]\r\nname=Alice\r\n";
    let output = render_config(&ini(), Some(original), &[setting("enabled", json!(true))]).unwrap();
    assert_eq!(output, original.replace("enabled\\default=false", "enabled\\default=true"));
    assert_eq!(render_config(&ini(), Some(&output), &[setting("enabled", json!(true))]).unwrap(), output);
}
