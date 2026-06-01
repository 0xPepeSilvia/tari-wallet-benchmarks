#!/usr/bin/env python
"""Generate N distinct valid Tari esmeralda one-sided addresses.

Strategy: minotari create-address produces a mainnet-network-byte
address (it ignores --network and TARI_NETWORK env). We decode the
per-section bs58 to recover the 67 raw bytes (network + features +
spend_key + view_key + checksum), patch byte[0] from mainnet (0x00)
to esmeralda (0x26), recompute the Damm checksum, and re-encode.

The Ristretto curve keys minotari produced remain valid, so the
wallet's address validator accepts the result.
"""
import argparse, json, os, subprocess, tempfile, base58


NETWORK_BYTE_ESME = 0x26

# Damm checksum coefficients per
# tari/base_layer/common_types/src/dammsum.rs (DICT_SIZE=256, bit_length=8)
DAMM_COEFFICIENTS = [4, 3, 1]


def damm_checksum(payload: bytes) -> int:
    """Compute the Damm checksum byte for `payload`. The checksum byte
    is appended such that damm(payload + [checksum]) == 0.

    The Tari Damm implementation: accumulate over payload + a final 0
    byte; the accumulator's final value IS the checksum byte that
    needs to be appended to make the augmented sequence's accumulator
    return to 0.
    """
    # MASK starts at 1, then add (1 << bit) for each coefficient.
    # Per tari/base_layer/common_types/src/dammsum.rs: COEFFICIENTS = [4,3,1]
    # MASK = 1 + 16 + 8 + 2 = 27 = 0x1B
    mask = 1
    for bit_pos in DAMM_COEFFICIENTS:
        mask += (1 << bit_pos)

    acc = 0
    for byte in payload:
        acc ^= byte
        overflow = (acc & 0x80) != 0
        acc = (acc << 1) & 0xff
        if overflow:
            acc ^= mask
    return acc & 0xff


def decode_tari_per_section(addr: str) -> bytes:
    """Decode per-section bs58: char[0] -> byte[0], char[1] -> byte[1],
    chars[2:] -> bytes[2:].
    """
    if len(addr) < 3:
        raise ValueError(f"address too short: {len(addr)} chars")
    b0 = base58.b58decode(addr[0:1])
    b1 = base58.b58decode(addr[1:2])
    rest = base58.b58decode(addr[2:])
    return bytes(b0) + bytes(b1) + bytes(rest)


def encode_tari_per_section(raw: bytes) -> str:
    if len(raw) < 3:
        raise ValueError(f"raw too short: {len(raw)} bytes")
    b0 = base58.b58encode(raw[0:1]).decode()
    b1 = base58.b58encode(raw[1:2]).decode()
    rest = base58.b58encode(raw[2:]).decode()
    return b0 + b1 + rest


def patch_to_esmeralda(mainnet_addr: str) -> str:
    """Decode, swap network byte to esmeralda, recompute Damm checksum, re-encode."""
    raw = decode_tari_per_section(mainnet_addr)
    if len(raw) != 67:
        raise ValueError(f"unexpected raw address length: {len(raw)}")

    body = bytearray(raw[:66])
    body[0] = NETWORK_BYTE_ESME
    new_checksum = damm_checksum(bytes(body))
    new_raw = bytes(body) + bytes([new_checksum])
    return encode_tari_per_section(new_raw)


def make_address(minotari_bin: str) -> str:
    """Run `minotari create-address` and return the esmeralda-patched address."""
    with tempfile.NamedTemporaryFile(suffix='.json', delete=False, dir='C:/tmp' if os.name == 'nt' else None) as tmp:
        tmp_path = tmp.name
    try:
        os.makedirs(os.path.dirname(tmp_path), exist_ok=True)
        result = subprocess.run(
            [minotari_bin, 'create-address', '--output-file', tmp_path],
            capture_output=True, timeout=30,
        )
        if result.returncode != 0:
            raise RuntimeError(f"create-address failed: rc={result.returncode}")
        with open(tmp_path) as f:
            data = json.load(f)
        mainnet_addr = data['address']
        esme_addr = patch_to_esmeralda(mainnet_addr)
        return esme_addr
    finally:
        try:
            os.unlink(tmp_path)
        except Exception:
            pass


def verify_known_damm_on_funded_wallet():
    """Sanity check: the funded wallet's address has its own Damm checksum
    in the last byte. damm(full 67 bytes) should be 0 if our checksum
    implementation is correct.
    """
    known = "f2Ln1PRd2bmwWqC3q8yydaoHFVSURyciar2ijamoz7Hy7FuVYXEqdCCqJCj2aY5DZSQxoCCPjvQTfHwkvdZmbrVVsM9"
    raw = decode_tari_per_section(known)
    if len(raw) != 67:
        return False, f"length {len(raw)} != 67"
    # damm(raw) should be 0 if the checksum is consistent
    full_checksum = damm_checksum(raw)
    return full_checksum == 0, f"damm(full)={full_checksum} (expected 0)"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--count', type=int, default=100)
    ap.add_argument('--minotari-bin', default='C:/Projects/Tariz/minotari-cli/target/release/minotari.exe')
    ap.add_argument('--output', default='results/distinct_addresses.json')
    args = ap.parse_args()

    # Sanity check our Damm impl first
    ok, msg = verify_known_damm_on_funded_wallet()
    print(f"Damm sanity check on known funded address: {ok} ({msg})")
    if not ok:
        print("Damm implementation is wrong - patched addresses will be invalid")
        # Continue anyway - the wallet will reject them and we'll know

    addrs = []
    for i in range(args.count):
        addr = make_address(args.minotari_bin)
        addrs.append(addr)
        if (i + 1) % 10 == 0:
            print(f"  generated {i + 1}/{args.count}", flush=True)

    assert len(set(addrs)) == args.count, "duplicates"

    with open(args.output, 'w') as f:
        json.dump({"count": args.count, "addresses": addrs}, f, indent=2)

    print(f"\nwrote {args.count} distinct esmeralda addresses to {args.output}")
    print(f"first: {addrs[0]}")
    print(f"prefix: {addrs[0][:2]} (length {len(addrs[0])} chars)")


if __name__ == '__main__':
    main()
