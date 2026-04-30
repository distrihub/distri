use crate::{FileType, Part};

#[test]
fn part_file_serializes_with_part_type_file() {
    let part = Part::File(FileType::Bytes {
        bytes: "JVBERi0xLjQKJ...".to_string(),
        mime_type: "application/pdf".to_string(),
        name: Some("report.pdf".to_string()),
    });

    let json = serde_json::to_value(&part).unwrap();
    assert_eq!(json["part_type"], "file");
    assert_eq!(json["data"]["type"], "bytes");
    assert_eq!(json["data"]["mime_type"], "application/pdf");
    assert_eq!(json["data"]["name"], "report.pdf");
}

#[test]
fn part_file_round_trips_through_json() {
    let original = Part::File(FileType::Url {
        url: "https://example.com/doc.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        name: None,
    });

    let json = serde_json::to_string(&original).unwrap();
    let decoded: Part = serde_json::from_str(&json).unwrap();

    match decoded {
        Part::File(FileType::Url { url, mime_type, name }) => {
            assert_eq!(url, "https://example.com/doc.pdf");
            assert_eq!(mime_type, "application/pdf");
            assert_eq!(name, None);
        }
        _ => panic!("expected Part::File(Url)"),
    }
}

#[test]
fn part_file_type_name_is_file() {
    let part = Part::File(FileType::Bytes {
        bytes: String::new(),
        mime_type: "text/plain".to_string(),
        name: None,
    });
    assert_eq!(part.type_name(), "file");
}

#[test]
fn filetype_mime_type_accessor_works_for_both_variants() {
    let b = FileType::Bytes {
        bytes: String::new(),
        mime_type: "application/pdf".to_string(),
        name: None,
    };
    assert_eq!(b.mime_type(), "application/pdf");

    let u = FileType::Url {
        url: String::new(),
        mime_type: "text/csv".to_string(),
        name: None,
    };
    assert_eq!(u.mime_type(), "text/csv");
}
