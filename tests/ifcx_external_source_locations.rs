#![cfg(feature = "ifcx")]
use opencaddocument::ifccad::IfccadExternalSourceLocation as Location;
use opencaddocument::{
    external_source_location_from_cad_path as from_cad,
    external_source_location_to_cad_path as to_cad,
};

#[test]
fn paths_keep_direct_and_search_meaning_and_decode_only_once() {
    for (cad, uri) in [
        (r"..\Refs\grond plan.dwg", "../Refs/grond%20plan.dwg"),
        (r"C:\CAD\grondplan.dwg", "file:///C:/CAD/grondplan.dwg"),
        (
            r"\\server\share\grondplan.dwg",
            "file://server/share/grondplan.dwg",
        ),
        ("./grondplan.dwg", "./grondplan.dwg"),
        ("../Refs/a%20#é.dwg", "../Refs/a%2520%23%C3%A9.dwg"),
    ] {
        let expected = Location::UriReference { uri: uri.into() };
        assert_eq!(from_cad(cad).unwrap(), expected);
        assert_eq!(from_cad(&to_cad(&expected).unwrap()).unwrap(), expected);
    }
    assert_eq!(
        from_cad("grondplan.dwg").unwrap(),
        Location::SearchFileName {
            file_name: "grondplan.dwg".into()
        }
    );
    assert_eq!(
        to_cad(&Location::UriReference {
            uri: "grondplan.dwg".into()
        })
        .unwrap(),
        "./grondplan.dwg"
    );
    assert_eq!(
        to_cad(&Location::UriReference {
            uri: "../a%2520.dwg".into()
        })
        .unwrap(),
        "../a%20.dwg"
    );
}

#[test]
fn valid_native_uris_without_cad_equivalent_have_explicit_errors() {
    for uri in [
        "https://example.org/a.dwg",
        "urn:uuid:e5b4a9af-cf20-4fc9-a30f-ffb20fd987e1",
        "a.dwg?version=2",
        "a.dwg#part",
        "file:///a%00.dwg",
    ] {
        assert!(
            to_cad(&Location::UriReference { uri: uri.into() }).is_err(),
            "{uri}"
        );
    }
    for path in ["", "C:relative.dwg", "bad\0.dwg"] {
        assert!(from_cad(path).is_err());
    }
}
