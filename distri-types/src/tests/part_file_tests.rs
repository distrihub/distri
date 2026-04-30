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

use distri_a2a::{FileObject as A2AFileObject, FilePart as A2AFilePart, Part as A2APart, Message as A2AMessage};

#[test]
fn a2a_pdf_inbound_becomes_part_file() {
    let a2a_part = A2APart::File(A2AFilePart {
        file: A2AFileObject::WithBytes {
            bytes: "JVBERi0xLjQK".to_string(),
            mime_type: Some("application/pdf".to_string()),
            name: Some("doc.pdf".to_string()),
        },
        metadata: None,
    });

    let a2a_msg = A2AMessage {
        parts: vec![a2a_part],
        ..Default::default()
    };

    let distri_msg: crate::Message = a2a_msg.try_into().unwrap();
    assert_eq!(distri_msg.parts.len(), 1);
    match &distri_msg.parts[0] {
        Part::File(FileType::Bytes { bytes, mime_type, name }) => {
            assert_eq!(bytes, "JVBERi0xLjQK");
            assert_eq!(mime_type, "application/pdf");
            assert_eq!(name.as_deref(), Some("doc.pdf"));
        }
        other => panic!("expected Part::File(Bytes), got {:?}", other),
    }
}

#[test]
fn a2a_image_inbound_still_becomes_part_image() {
    let a2a_part = A2APart::File(A2AFilePart {
        file: A2AFileObject::WithUri {
            uri: "https://example.com/cat.png".to_string(),
            mime_type: Some("image/png".to_string()),
            name: None,
        },
        metadata: None,
    });

    let a2a_msg = A2AMessage {
        parts: vec![a2a_part],
        ..Default::default()
    };

    let distri_msg: crate::Message = a2a_msg.try_into().unwrap();
    match &distri_msg.parts[0] {
        Part::Image(FileType::Url { url, mime_type, .. }) => {
            assert_eq!(url, "https://example.com/cat.png");
            assert_eq!(mime_type, "image/png");
        }
        other => panic!("expected Part::Image(Url), got {:?}", other),
    }
}

#[test]
fn a2a_unknown_mime_inbound_becomes_part_file_no_error() {
    let a2a_part = A2APart::File(A2AFilePart {
        file: A2AFileObject::WithBytes {
            bytes: "AAAA".to_string(),
            mime_type: None,
            name: None,
        },
        metadata: None,
    });

    let a2a_msg = A2AMessage {
        parts: vec![a2a_part],
        ..Default::default()
    };

    let distri_msg: crate::Message = a2a_msg.try_into().unwrap();
    match &distri_msg.parts[0] {
        Part::File(_) => {}
        other => panic!("expected Part::File for unknown mime, got {:?}", other),
    }
}

#[test]
fn distri_part_file_outbound_becomes_a2a_file() {
    let distri_part = Part::File(FileType::Url {
        url: "https://example.com/r.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        name: Some("r.pdf".to_string()),
    });

    let a2a_part: A2APart = distri_part.into();
    match a2a_part {
        A2APart::File(fp) => match fp.file {
            A2AFileObject::WithUri { uri, mime_type, name } => {
                assert_eq!(uri, "https://example.com/r.pdf");
                assert_eq!(mime_type.as_deref(), Some("application/pdf"));
                assert_eq!(name.as_deref(), Some("r.pdf"));
            }
            _ => panic!("expected WithUri"),
        },
        _ => panic!("expected A2APart::File"),
    }
}
