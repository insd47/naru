using System.Runtime.InteropServices;
using System.Text;

namespace Naru.Store;

/// <summary>
/// Stores saved Macs' passwords in Windows Credential Manager.
/// </summary>
/// <remarks>
/// Credentials are saved with <c>CRED_PERSIST_LOCAL_MACHINE</c>, so they stay on this PC. <c>PasswordVault</c> is avoided
/// because it roams with the Microsoft account, and enterprise persistence roams with domain profiles.
/// </remarks>
internal static partial class Keychain
{
    private const uint CredTypeGeneric = 1;
    private const uint CredPersistLocalMachine = 2;
    private const int ErrorNotFound = 1168;

    public static string? Find(string hostId)
    {
        if (!CredRead(Target(hostId), CredTypeGeneric, 0, out var credential))
        {
            return null;
        }

        try
        {
            unsafe
            {
                var record = (Credential*)credential;
                return Encoding.Unicode.GetString(new ReadOnlySpan<byte>((void*)record->CredentialBlob, (int)record->CredentialBlobSize));
            }
        }
        finally
        {
            CredFree(credential);
        }
    }

    public static void Save(string hostId, string username, string password)
    {
        var blob = Encoding.Unicode.GetBytes(password);

        try
        {
            unsafe
            {
                fixed (char* target = Target(hostId))
                fixed (char* user = username)
                fixed (byte* secret = blob)
                {
                    var credential = new Credential
                    {
                        Type = CredTypeGeneric,
                        TargetName = (nint)target,
                        CredentialBlobSize = (uint)blob.Length,
                        CredentialBlob = (nint)secret,
                        Persist = CredPersistLocalMachine,
                        UserName = (nint)user,
                    };

                    if (!CredWrite(ref credential, 0))
                    {
                        throw new System.ComponentModel.Win32Exception(Marshal.GetLastPInvokeError());
                    }
                }
            }
        }
        finally
        {
            Array.Clear(blob);
        }
    }

    public static void Remove(string hostId)
    {
        if (!CredDelete(Target(hostId), CredTypeGeneric, 0) && Marshal.GetLastPInvokeError() != ErrorNotFound)
        {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastPInvokeError());
        }
    }

    private static string Target(string hostId) => $"Naru/{hostId}";

    [StructLayout(LayoutKind.Sequential)]
    private struct Credential
    {
        public uint Flags;
        public uint Type;
        public nint TargetName;
        public nint Comment;
        public long LastWritten;
        public uint CredentialBlobSize;
        public nint CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public nint Attributes;
        public nint TargetAlias;
        public nint UserName;
    }

    [LibraryImport("advapi32.dll", EntryPoint = "CredReadW", StringMarshalling = StringMarshalling.Utf16, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CredRead(string target, uint type, uint flags, out nint credential);

    [LibraryImport("advapi32.dll", EntryPoint = "CredWriteW", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CredWrite(ref Credential credential, uint flags);

    [LibraryImport("advapi32.dll", EntryPoint = "CredDeleteW", StringMarshalling = StringMarshalling.Utf16, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CredDelete(string target, uint type, uint flags);

    [LibraryImport("advapi32.dll")]
    private static partial void CredFree(nint buffer);
}
