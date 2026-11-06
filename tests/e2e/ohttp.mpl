# Preparation-only public API fixture. Not compiled or executed yet.
fn report(name :: String, passed :: Bool) do
  if passed do println(name <> ":ok") else println(name <> ":failed") end
end

fn qualified_roundtrip() -> Bool!CryptoError do
  let pair = Crypto.x25519_generate()?
  let body = Bytes.from_utf8("binary request\0")
  let response = Bytes.from_utf8("binary response\0")
  let outgoing = Ohttp.encapsulate_request(37, pair.public_key, body)?
  let wire = outgoing.wire
  let incoming = Ohttp.decapsulate_request(pair.private_key, 37, wire)?
  let request_matches = Bytes.secure_equals(incoming.plaintext, body)
  let reply = Ohttp.encapsulate_response(incoming.response_context, response)?
  let opened = Ohttp.decapsulate_response(outgoing.response_context, reply)?
  let passed = request_matches && Bytes.secure_equals(opened, response) && Bytes.length(wire) == Bytes.length(body) + 55 && Bytes.length(reply) == Bytes.length(response) + 48
  Secret.destroy(pair.private_key)
  Ok(passed)
end

fn flat_roundtrip() -> Bool!CryptoError do
  let pair = Crypto.x25519_generate()?
  let body = Bytes.from_utf8("binary request\0")
  let response = Bytes.from_utf8("binary response\0")
  let outgoing = ohttp_encapsulate_request(37, pair.public_key, body)?
  let wire = outgoing.wire
  let incoming = ohttp_decapsulate_request(pair.private_key, 37, wire)?
  let request_matches = Bytes.secure_equals(incoming.plaintext, body)
  let reply = ohttp_encapsulate_response(incoming.response_context, response)?
  let opened = ohttp_decapsulate_response(outgoing.response_context, reply)?
  let passed = request_matches && Bytes.secure_equals(opened, response) && Bytes.length(wire) == Bytes.length(body) + 55 && Bytes.length(reply) == Bytes.length(response) + 48
  Secret.destroy(pair.private_key)
  Ok(passed)
end

fn id_bounds() -> Bool!CryptoError do
  let pair = Crypto.x25519_generate()?
  let negative = case Ohttp.encapsulate_request(-1, pair.public_key, Bytes.empty()) do
    Err(InvalidLength(expected, actual)) -> expected == 255 && actual == -1
    _ -> false
  end
  let too_large = case ohttp_encapsulate_request(256, pair.public_key, Bytes.empty()) do
    Err(InvalidLength(expected, actual)) -> expected == 255 && actual == 256
    _ -> false
  end
  Secret.destroy(pair.private_key)
  Ok(negative && too_large)
end

fn independent_request() -> Bool!CryptoError do
  let pair = Crypto.x25519_from_seed(Result.unwrap_or(Bytes.from_hex("ef0106ca50fd40bd29f8e1cfb6079fd37b4052d3bf4ce20b616aff8dd2bd8c5f"), Bytes.empty()))?
  let wire = Result.unwrap_or(Bytes.from_hex("25002000010003f0475110cd1e58fa47525136677c4a9febd94b41a123f4adcd1f7eaaef469159ac8dfab44fa4ecc1f50d28e64a4b9b6f97a1ede786789fb17cb6c5835b8bed25f82e069581c300b3ea82e8fb"), Bytes.empty())
  let expected = Result.unwrap_or(Bytes.from_hex("00696e646570656e64656e742062696e6172792072657175657374ff"), Bytes.empty())
  let opened = Ohttp.decapsulate_request(pair.private_key, 37, wire)?
  let passed = Bytes.secure_equals(opened.plaintext, expected)
  Secret.destroy(opened.response_context)
  Secret.destroy(pair.private_key)
  Ok(passed)
end

fn complete_wire_bounds() -> Bool!CryptoError do
  let pair = Crypto.x25519_generate()?
  let body = Result.unwrap_or(Bytes.repeat(165, 1048521), Bytes.empty())
  let response = Result.unwrap_or(Bytes.repeat(90, 1048528), Bytes.empty())
  let request = Ohttp.encapsulate_request(37, pair.public_key, body)?
  let wire = request.wire
  let incoming = Ohttp.decapsulate_request(pair.private_key, 37, wire)?
  let request_matches = Bytes.length(wire) == 1048576 && Bytes.secure_equals(incoming.plaintext, body)
  let reply = Ohttp.encapsulate_response(incoming.response_context, response)?
  let opened = Ohttp.decapsulate_response(request.response_context, reply)?
  let passed = request_matches && Bytes.length(reply) == 1048576 && Bytes.secure_equals(opened, response)
  Secret.destroy(pair.private_key)
  Ok(passed)
end

fn main() do
  report("ohttp:qualified", Result.unwrap_or(qualified_roundtrip(), false))
  report("ohttp:flat", Result.unwrap_or(flat_roundtrip(), false))
  report("ohttp:bounds", Result.unwrap_or(id_bounds(), false))
  report("ohttp:reference-request", Result.unwrap_or(independent_request(), false))
  report("ohttp:wire-caps", Result.unwrap_or(complete_wire_bounds(), false))
end
