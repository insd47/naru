import SwiftUI

/// The sheet for editing a saved Mac, following Screen Sharing's edit window.
///
/// - An empty name is filled with the Mac's computer name on the next connection.
/// - Connect saves the edits and connects right away.
struct EditSheet: View {
    let original: Host
    let onSave: (Host, _ connect: Bool) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var host: Host
    @State private var port: String

    init(original: Host, onSave: @escaping (Host, Bool) -> Void) {
        self.original = original
        self.onSave = onSave
        _host = State(initialValue: original)
        _port = State(initialValue: String(original.port))
    }

    private var valid: Bool {
        !host.address.isEmpty && Int(port).map { (1...65535).contains($0) } == true
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $host.name, prompt: Text("Optional"))
                    TextField("Server Address", text: $host.address, prompt: Text("Hostname or IP address"))
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Username", text: $host.username)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }

                Section {
                    Picker("Display Type", selection: $host.virtualDisplay) {
                        Text("Virtual Display").tag(true)
                        Text("Mac's Display").tag(false)
                    }
                    TextField("Port", text: $port)
                        .keyboardType(.numberPad)
                } footer: {
                    if let last = original.lastConnected {
                        Text("Last connected \(formatDate(last))")
                    }
                }
            }
            .navigationTitle("Edit “\(original.title)”")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", role: .cancel) { dismiss() }
                }
                ToolbarItemGroup(placement: .confirmationAction) {
                    Button("Connect") { save(connect: true) }.disabled(!valid)
                    Button("Done") { save(connect: false) }.disabled(!valid)
                }
            }
        }
    }

    private func save(connect: Bool) {
        var edited = host
        edited.name = edited.name.trimmingCharacters(in: .whitespaces)
        edited.address = edited.address.trimmingCharacters(in: .whitespaces)
        edited.username = edited.username.trimmingCharacters(in: .whitespaces)
        edited.port = Int(port) ?? edited.port
        onSave(edited, connect)
        dismiss()
    }
}

/// The sheet that asks for the user name and password when connecting. A wrong password is reported through [message].
struct LoginSheet: View {
    let host: Host
    let message: String?
    let onSignIn: (SessionController.Credentials) -> Void
    let onCancel: () -> Void

    @State private var username: String
    @State private var password = ""
    @State private var remember = true

    init(host: Host, message: String?, onSignIn: @escaping (SessionController.Credentials) -> Void, onCancel: @escaping () -> Void) {
        self.host = host
        self.message = message
        self.onSignIn = onSignIn
        self.onCancel = onCancel
        _username = State(initialValue: host.username)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $username)
                        .textContentType(.username)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField("Password", text: $password)
                        .textContentType(.password)
                        .onSubmit(signIn)
                    Toggle("Remember this password", isOn: $remember)
                } header: {
                    Text("Enter your name and password to share this Mac's screen.")
                } footer: {
                    if let message {
                        Text(message).foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Sign In to “\(host.title)”")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", role: .cancel, action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Sign In", action: signIn).disabled(username.isEmpty || password.isEmpty)
                }
            }
        }
    }

    private func signIn() {
        guard !username.isEmpty, !password.isEmpty else { return }
        onSignIn(.init(username: username.trimmingCharacters(in: .whitespaces), password: password, remember: remember))
    }
}
