import Foundation

@main
struct Host {
  static func main() throws {
    try MeshLibrary.initialize()
    defer { MeshLibrary.shutdown() }
    do {
      // No secure-store callback is installed in this host.
      _ = try MeshLibrary.storage_roundtrip(Data([1]))
      fatalError("Expected a platform storage error")
    } catch let failure as MeshLibraryFailure {
      precondition(failure.status == MESH_LIBRARY_ERR_APPLICATION)
      let expected = "Mesh library call failed (status=9): platform storage key failed"
      precondition(failure.localizedDescription == expected, failure.localizedDescription)
      precondition((failure as NSError).localizedDescription == expected)
    }
    for payload in [Data(), Data([0xff])] {
      let failure = MeshLibraryFailure(status: MESH_LIBRARY_ERR_NOT_INITIALIZED, payload: payload)
      precondition(failure.localizedDescription == "Mesh library call failed (status=2)")
    }
    let unicode = MeshLibraryFailure(status: 9, payload: Data("échec".utf8))
    precondition(unicode.localizedDescription == "Mesh library call failed (status=9): échec")
  }
}
