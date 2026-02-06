use pyrs::bytecode::pyc::{PycHeader, parse_pyc_header, write_pyc_header};

#[test]
fn parses_hash_based_header() {
    let bytes = [
        1, 0, 0, 0, // magic
        1, 0, 0, 0, // bitfield (hash-based)
        1, 2, 3, 4, 5, 6, 7, 8, // hash
    ];

    let (header, offset) = parse_pyc_header(&bytes).expect("parse should succeed");
    assert_eq!(header.magic, 1);
    assert_eq!(header.bitfield, 1);
    assert_eq!(header.hash, Some([1, 2, 3, 4, 5, 6, 7, 8]));
    assert!(header.timestamp.is_none());
    assert!(header.source_size.is_none());
    assert_eq!(offset, 16);
}

#[test]
fn parses_timestamp_based_header() {
    let bytes = [
        2, 0, 0, 0, // magic
        0, 0, 0, 0, // bitfield
        3, 0, 0, 0, // timestamp
        4, 0, 0, 0, // source size
    ];

    let (header, offset) = parse_pyc_header(&bytes).expect("parse should succeed");
    assert_eq!(header.magic, 2);
    assert_eq!(header.bitfield, 0);
    assert_eq!(header.timestamp, Some(3));
    assert_eq!(header.source_size, Some(4));
    assert!(header.hash.is_none());
    assert_eq!(offset, 16);
}

#[test]
fn writes_hash_based_header() {
    let header = PycHeader {
        magic: 0x0A0D0DFA,
        bitfield: 1,
        timestamp: None,
        source_size: None,
        hash: Some([1, 2, 3, 4, 5, 6, 7, 8]),
    };
    let mut bytes = Vec::new();
    write_pyc_header(&header, &mut bytes).expect("write should succeed");
    let (parsed, offset) = parse_pyc_header(&bytes).expect("parse should succeed");
    assert_eq!(parsed, header);
    assert_eq!(offset, 16);
}

#[test]
fn writes_timestamp_based_header() {
    let header = PycHeader {
        magic: 0x0A0D0DFA,
        bitfield: 0,
        timestamp: Some(123),
        source_size: Some(456),
        hash: None,
    };
    let mut bytes = Vec::new();
    write_pyc_header(&header, &mut bytes).expect("write should succeed");
    let (parsed, offset) = parse_pyc_header(&bytes).expect("parse should succeed");
    assert_eq!(parsed, header);
    assert_eq!(offset, 16);
}
