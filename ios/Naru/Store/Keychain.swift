import Foundation
import Security

/// 저장한 Mac의 비밀번호를 키체인에 둡니다. 이 기기에만 남고 백업으로 옮겨지지 않습니다.
enum Keychain {
    static func find(_ hostId: String) -> String? {
        var query = item(hostId)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let data = result as? Data else {
            return nil
        }

        return String(data: data, encoding: .utf8)
    }

    static func save(_ hostId: String, password: String) {
        remove(hostId)

        var query = item(hostId)
        query[kSecValueData as String] = Data(password.utf8)
        query[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        SecItemAdd(query as CFDictionary, nil)
    }

    static func remove(_ hostId: String) {
        SecItemDelete(item(hostId) as CFDictionary)
    }

    private static func item(_ hostId: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "dev.insd.naru.password", kSecAttrAccount as String: hostId]
    }
}
