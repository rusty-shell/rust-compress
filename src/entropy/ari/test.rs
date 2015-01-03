use std::io::{BufReader, BufWriter, MemWriter, SeekSet};
use std::iter::repeat;
use std::vec::Vec;
use test::Bencher;

static TEXT_INPUT: &'static [u8] = include_bytes!("../../data/test.txt");


fn roundtrip(bytes: &[u8]) {
    info!("Roundtrip Ari of size {}", bytes.len());
    let mut e = super::table::ByteEncoder::new(MemWriter::new());
    e.write(bytes).unwrap();
    let (e, r) = e.finish();
    r.unwrap();
    let encoded = e.into_inner();
    debug!("Roundtrip input {} encoded {}", bytes, encoded);
    let mut d = super::ByteDecoder::new(BufReader::new(encoded.as_slice()));
    let decoded = d.read_to_end().unwrap();
    assert_eq!(bytes.as_slice(), decoded.as_slice());
}

fn encode_binary(bytes: &[u8], model: &mut super::bin::Model) -> Vec<u8> {
    let mut encoder = super::Encoder::new(MemWriter::new());
    for &byte in bytes.iter() {
        for i in range(0u,8) {
            let bit = (byte & (1<<i)) != 0;
            encoder.encode(bit, model).unwrap();
            model.update(bit);
        }
    }
    let (writer, err) = encoder.finish();
    err.unwrap();
    writer.into_inner()
}

fn roundtrip_binary(bytes: &[u8], factor: u32) {
    let mut bm = super::bin::Model::new_flat(super::RANGE_DEFAULT_THRESHOLD >> 3, factor);
    let output = encode_binary(bytes, &mut bm);
    bm.reset_flat();
    let mut decoder = super::Decoder::new(BufReader::new(output.as_slice()));
    for &byte in bytes.iter() {
        let mut value = 0u8;
        for i in range(0u,8) {
            let bit = decoder.decode(&bm).unwrap();
            bm.update(bit);
            value += (bit as u8)<<i;
        }
        assert_eq!(value, byte);
    }
}

fn roundtrip_term(bytes1: &[u8], bytes2: &[u8]) {
    let mw = MemWriter::new();
    let mw = {
        let mut e = super::table::ByteEncoder::new(mw);
        e.write(bytes1).unwrap();
        let (stream, rez) = e.finish();
        rez.unwrap();
        stream
    };
    let mw = {
        let mut e = super::table::ByteEncoder::new(mw);
        e.write(bytes2).unwrap();
        let (stream, rez) = e.finish();
        rez.unwrap();
        stream
    };
    let encoded = mw.into_inner();
    debug!("Roundtrip term input {}:{} encoded {}", bytes1, bytes2, encoded);
    let br = BufReader::new(encoded.as_slice());
    let br = {
        let mut d = super::ByteDecoder::new(br);
        let decoded = d.read_to_end().unwrap();
        assert_eq!(bytes1.as_slice(), decoded.as_slice());
        let (stream, err) = d.finish();
        err.unwrap();
        stream
    };
    {
        let mut d = super::ByteDecoder::new(br);
        let decoded = d.read_to_end().unwrap();
        assert_eq!(bytes2.as_slice(), decoded.as_slice());
        let (stream, err) = d.finish();
        err.unwrap();
        stream
    };
}

fn roundtrip_proxy(bytes: &[u8]) {
    // prepare data
    let update0 = 10;
    let update1 = 5;
    let threshold = super::RANGE_DEFAULT_THRESHOLD >> 3;
    let mut t0 = super::table::Model::new_flat(16, threshold);
    let mut t1 = super::table::Model::new_flat(16, threshold);
    let mut b0 = super::bin::Model::new_flat(threshold, 3);
    let mut b1 = super::bin::Model::new_flat(threshold, 5);
    // encode (high 4 bits with the proxy table, low 4 bits with the proxy binary)
    let mut encoder = super::Encoder::new(MemWriter::new());
    for &byte in bytes.iter() {
        let high = (byte>>4) as uint;
        {
            let proxy = super::table::SumProxy::new(2, &t0, 1, &t1, 0);
            encoder.encode(high, &proxy).unwrap();
        }
        t0.update(high, update0, 1);
        t1.update(high, update1, 1);
        for i in range(0u,4) {
            let bit = (byte & (1<<i)) != 0;
            {
                let proxy = super::bin::SumProxy::new(1, &b0, 1, &b1, 1);
                encoder.encode(bit, &proxy).unwrap();
            }
            b0.update(bit);
            b1.update(bit);
        }
    }
    let (writer, err) = encoder.finish();
    err.unwrap();
    let buffer = writer.into_inner();
    // decode
    t0.reset_flat();
    t1.reset_flat();
    b0.reset_flat();
    b1.reset_flat();
    let mut decoder = super::Decoder::new(BufReader::new(buffer.as_slice()));
    for &byte in bytes.iter() {
        let high = {
            let proxy = super::table::SumProxy::new(2, &t0, 1, &t1, 0);
            decoder.decode(&proxy).unwrap()
        };
        t0.update(high, update0, 1);
        t1.update(high, update1, 1);
        let mut value = (high<<4) as u8;
        for i in range(0u,4) {
            let bit = {
                let proxy = super::bin::SumProxy::new(1, &b0, 1, &b1, 1);
                decoder.decode(&proxy).unwrap()
            };
            value += (bit as u8)<<i;
            b0.update(bit);
            b1.update(bit);
        }
        assert_eq!(value, byte);
    }
}

fn roundtrip_apm(bytes: &[u8]) {
    let mut bit = super::apm::Bit::new_equal();
    let mut gate = super::apm::Gate::new();
    let mut encoder = super::Encoder::new(MemWriter::new());
    for b8 in bytes.iter() {
        for i in range(0u,8) {
            let b1 = (*b8>>i) & 1 != 0;
            let (bit_new, coords) = gate.pass(&bit);
            encoder.encode(b1, &bit_new).unwrap();
            bit.update(b1, 10, 0);
            gate.update(b1, coords, 10, 0);
        }
    }
    let (writer, err) = encoder.finish();
    err.unwrap();
    let output = writer.into_inner();
    bit = super::apm::Bit::new_equal();
    gate = super::apm::Gate::new();
    let mut decoder = super::Decoder::new(BufReader::new(output.as_slice()));
    for b8 in bytes.iter() {
        let mut decoded = 0u8;
        for i in range(0u,8) {
            let (bit_new, coords) = gate.pass(&bit);
            let b1 = decoder.decode(&bit_new).unwrap();
            if b1 {
                decoded += 1<<i;
            }
            bit.update(b1, 10, 0);
            gate.update(b1, coords, 10, 0);
        }
        assert_eq!(decoded, *b8);
    }
}


#[test]
fn roundtrips() {
    roundtrip(b"abracadabra");
    roundtrip(b"");
    roundtrip(TEXT_INPUT);
}

#[test]
fn roundtrips_binary() {
    roundtrip_binary(b"abracadabra", 1);
    roundtrip_binary(TEXT_INPUT, 5);
}

#[test]
fn roundtrips_term() {
    roundtrip_term(b"abra", b"cadabra");
}

#[test]
fn roundtrips_proxy() {
    roundtrip_proxy(b"abracadabra");
    roundtrip_proxy(TEXT_INPUT);
}

#[test]
fn roundtrips_apm() {
    roundtrip_apm(b"abracadabra");
}

#[bench]
fn compress_speed(bh: &mut Bencher) {
    let mut storage: Vec<u8> = repeat(0u8).take(TEXT_INPUT.len()).collect();
    bh.iter(|| {
        let mut w = BufWriter::new(storage.as_mut_slice());
        w.seek(0, SeekSet).unwrap();
        let mut e = super::ByteEncoder::new(w);
        e.write(TEXT_INPUT).unwrap();
    });
    bh.bytes = TEXT_INPUT.len() as u64;
}
