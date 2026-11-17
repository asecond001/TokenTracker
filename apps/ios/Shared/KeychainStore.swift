import Foundation
import Security

public protocol SecretStoring: Sendable {
    func saveSecret(_ value: String, account: KeychainSecretAccount) throws
    func loadSecret(account: KeychainSecretAccount) throws -> String?
    func deleteSecret(account: KeychainSecretAccount) throws
}

public enum KeychainSecretAccount: String, CaseIterable, Sendable {
    case syncBearerToken = "sync_bearer_token"
    case e2eePassphrase = "e2ee_passphrase"
}

public enum KeychainStoreError: LocalizedError, Equatable {
    case encodingFailed
    case unexpectedStatus(OSStatus)

    public var errorDescription: String? {
        switch self {
        case .encodingFailed:
            return "Failed to encode secret payload as UTF-8"
        case .unexpectedStatus(let status):
            return "Keychain operation failed with status \(status)"
        }
    }
}

public final class KeychainStore: SecretStoring, @unchecked Sendable {
    public static let shared = KeychainStore()

    private let service: String

    public init(service: String = "com.asecond.tokenvane.sync") {
        self.service = service
    }

    public func saveSecret(_ value: String, account: KeychainSecretAccount) throws {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            try deleteSecret(account: account)
            return
        }
        guard let encoded = trimmed.data(using: .utf8) else {
            throw KeychainStoreError.encodingFailed
        }

        let baseQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account.rawValue
        ]

        let attributes: [String: Any] = [
            kSecValueData as String: encoded,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]

        let updateStatus = SecItemUpdate(baseQuery as CFDictionary, attributes as CFDictionary)
        if updateStatus == errSecSuccess {
            return
        }
        if updateStatus != errSecItemNotFound {
            throw KeychainStoreError.unexpectedStatus(updateStatus)
        }

        var addQuery = baseQuery
        addQuery[kSecValueData as String] = encoded
        addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly

        let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
        guard addStatus == errSecSuccess else {
            throw KeychainStoreError.unexpectedStatus(addStatus)
        }
    }

    public func loadSecret(account: KeychainSecretAccount) throws -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account.rawValue,
            kSecReturnData as String: kCFBooleanTrue as Any,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw KeychainStoreError.unexpectedStatus(status)
        }
        guard let data = result as? Data else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    public func deleteSecret(account: KeychainSecretAccount) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account.rawValue
        ]
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainStoreError.unexpectedStatus(status)
        }
    }
}

public final class InMemorySecretStore: SecretStoring, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [KeychainSecretAccount: String] = [:]

    public init(initial: [KeychainSecretAccount: String] = [:]) {
        self.values = initial
    }

    public func saveSecret(_ value: String, account: KeychainSecretAccount) throws {
        lock.lock()
        defer { lock.unlock() }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            values.removeValue(forKey: account)
        } else {
            values[account] = trimmed
        }
    }

    public func loadSecret(account: KeychainSecretAccount) throws -> String? {
        lock.lock()
        defer { lock.unlock() }
        return values[account]
    }

    public func deleteSecret(account: KeychainSecretAccount) throws {
        lock.lock()
        defer { lock.unlock() }
        values.removeValue(forKey: account)
    }
}
