// Maintainer-only release utility. Private key bytes stay in Keychain and
// process memory; only the public key and detached signatures are written.
import Foundation
import Security
import CryptoKit

enum SigningError: Error { case invalidArguments, keychain(OSStatus), missingKey, publicKeyMismatch }
let service = "LawPDF release manifest signing v1"
let account = "lawpdf-maintainer"

func signingKey(create: Bool) throws -> Curve25519.Signing.PrivateKey {
    let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service, kSecAttrAccount as String: account,
        kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitOne]
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    if status == errSecSuccess, let data = result as? Data {
        return try Curve25519.Signing.PrivateKey(rawRepresentation: data)
    }
    guard status == errSecItemNotFound else { throw SigningError.keychain(status) }
    guard create else { throw SigningError.missingKey }
    let key = Curve25519.Signing.PrivateKey()
    let attributes: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service, kSecAttrAccount as String: account,
        kSecAttrLabel as String: "LawPDF update signing (private)",
        kSecValueData as String: key.rawRepresentation,
        kSecAttrSynchronizable as String: false]
    let added = SecItemAdd(attributes as CFDictionary, nil)
    guard added == errSecSuccess else { throw SigningError.keychain(added) }
    // Confirm retrieval before publishing a trust root.
    return try signingKey(create: false)
}

func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }
do {
    let args = Array(CommandLine.arguments.dropFirst())
    guard args.count >= 2 else { throw SigningError.invalidArguments }
    if args[0] == "init", args.count == 2 {
        let key = try signingKey(create: true)
        let encoded = hex(key.publicKey.rawRepresentation) + "\n"
        let path = URL(fileURLWithPath: args[1])
        if FileManager.default.fileExists(atPath: path.path) {
            guard try String(contentsOf: path, encoding: .utf8) == encoded else { throw SigningError.publicKeyMismatch }
        } else {
            try encoded.write(to: path, atomically: true, encoding: .utf8)
        }
        print("Public update key is ready. Private key remains in Keychain.")
    } else if args[0] == "sign", args.count == 4 {
        let key = try signingKey(create: false)
        let trusted = try String(contentsOfFile: args[1], encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
        guard trusted == hex(key.publicKey.rawRepresentation) else { throw SigningError.publicKeyMismatch }
        let message = try Data(contentsOf: URL(fileURLWithPath: args[2]))
        guard message.count <= 262144 else { throw SigningError.invalidArguments }
        let signature = try key.signature(for: message)
        try (hex(signature) + "\n").write(toFile: args[3], atomically: true, encoding: .utf8)
        print("Release manifest signature written.")
    } else { throw SigningError.invalidArguments }
} catch {
    // Never print key material or arbitrary platform error payloads.
    fputs("Signing did not finish. Check arguments, Keychain access, and the pinned public key.\n", stderr)
    exit(1)
}
