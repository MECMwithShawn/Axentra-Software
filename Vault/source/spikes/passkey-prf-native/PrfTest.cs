// Native Windows WebAuthn PRF test: evaluates the browser test's passkey through webauthn.dll
// and prints the same fingerprint format, so the two paths can be compared.
// Build: csc /platform:x64 /out:PrfTest.exe PrfTest.cs
// Run:   PrfTest.exe <credId base64> <salt base64> [prf|raw]
using System;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;

[StructLayout(LayoutKind.Sequential)]
struct ClientData { public uint dwVersion; public uint cbClientDataJSON; public IntPtr pbClientDataJSON; public IntPtr pwszHashAlgId; }

[StructLayout(LayoutKind.Sequential)]
struct Credential { public uint dwVersion; public uint cbId; public IntPtr pbId; public IntPtr pwszCredentialType; }

[StructLayout(LayoutKind.Sequential)]
struct Credentials { public uint cCredentials; public IntPtr pCredentials; }

[StructLayout(LayoutKind.Sequential)]
struct Extensions { public uint cExtensions; public IntPtr pExtensions; }

[StructLayout(LayoutKind.Sequential)]
struct HmacSecretSalt { public uint cbFirst; public IntPtr pbFirst; public uint cbSecond; public IntPtr pbSecond; }

[StructLayout(LayoutKind.Sequential)]
struct HmacSecretSaltValues { public IntPtr pGlobalHmacSalt; public uint cCredWithHmacSecretSaltList; public IntPtr pCredWithHmacSecretSaltList; }

// WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS through version 6.
[StructLayout(LayoutKind.Sequential)]
struct GetAssertionOptions {
    public uint dwVersion;
    public uint dwTimeoutMilliseconds;
    public Credentials CredentialList;
    public Extensions Extensions;
    public uint dwAuthenticatorAttachment;
    public uint dwUserVerificationRequirement;
    public uint dwFlags;
    public IntPtr pwszU2fAppId;           // v2
    public IntPtr pbU2fAppId;
    public IntPtr pCancellationId;        // v3
    public IntPtr pAllowCredentialList;   // v4
    public uint dwCredLargeBlobOperation; // v5
    public uint cbCredLargeBlob;
    public IntPtr pbCredLargeBlob;
    public IntPtr pHmacSecretSaltValues;  // v6
    public int bBrowserInPrivateMode;
}

// WEBAUTHN_ASSERTION through version 5.
[StructLayout(LayoutKind.Sequential)]
struct Assertion {
    public uint dwVersion;
    public uint cbAuthenticatorData;
    public IntPtr pbAuthenticatorData;
    public uint cbSignature;
    public IntPtr pbSignature;
    public Credential Credential;
    public uint cbUserId;
    public IntPtr pbUserId;
    public Extensions Extensions;         // v2
    public uint cbCredLargeBlob;          // v3
    public IntPtr pbCredLargeBlob;
    public uint dwCredLargeBlobStatus;
    public IntPtr pHmacSecret;            // v4
    public uint dwUsedTransport;          // v5
}

static class Native {
    public const uint HMAC_SECRET_VALUES_FLAG = 0x00100000;

    [DllImport("webauthn.dll")] public static extern uint WebAuthNGetApiVersionNumber();
    [DllImport("webauthn.dll", CharSet = CharSet.Unicode)]
    public static extern int WebAuthNAuthenticatorGetAssertion(IntPtr hWnd, string rpId, ref ClientData clientData, ref GetAssertionOptions options, out IntPtr assertion);
    [DllImport("webauthn.dll")] public static extern void WebAuthNFreeAssertion(IntPtr assertion);
    [DllImport("webauthn.dll")] public static extern IntPtr WebAuthNGetErrorName(int hr);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}

static class PrfTest {
    static IntPtr Pin(byte[] data) {
        IntPtr p = Marshal.AllocHGlobal(data.Length);
        Marshal.Copy(data, 0, p, data.Length);
        return p;
    }

    static IntPtr PinStruct(object s) {
        IntPtr p = Marshal.AllocHGlobal(Marshal.SizeOf(s));
        Marshal.StructureToPtr(s, p, false);
        return p;
    }

    static string B64Url(byte[] b) {
        return Convert.ToBase64String(b).TrimEnd('=').Replace('+', '-').Replace('/', '_');
    }

    // Matches the browser page: first 16 hex chars of SHA-256(output).
    static string Fingerprint(byte[] b) {
        using (SHA256 sha = SHA256.Create()) {
            return BitConverter.ToString(sha.ComputeHash(b)).Replace("-", "").ToLowerInvariant().Substring(0, 16);
        }
    }

    // WebAuthn PRF -> hmac-secret salt: SHA-256("WebAuthn PRF" || 0x00 || input).
    static byte[] PrfSalt(byte[] input) {
        byte[] label = Encoding.ASCII.GetBytes("WebAuthn PRF");
        byte[] buf = new byte[label.Length + 1 + input.Length];
        Buffer.BlockCopy(label, 0, buf, 0, label.Length);
        Buffer.BlockCopy(input, 0, buf, label.Length + 1, input.Length);
        using (SHA256 sha = SHA256.Create()) { return sha.ComputeHash(buf); }
    }

    static int Main(string[] args) {
        if (args.Length < 2) {
            Console.WriteLine("usage: PrfTest.exe <credId base64> <salt base64> [prf|raw]");
            return 2;
        }
        byte[] credId = Convert.FromBase64String(args[0]);
        byte[] input = Convert.FromBase64String(args[1]);
        string mode = args.Length > 2 ? args[2] : "prf";

        uint api = Native.WebAuthNGetApiVersionNumber();
        Console.WriteLine("webauthn.dll API version: " + api);
        if (api < 4) { Console.WriteLine("API too old for hmac-secret/PRF."); return 1; }

        // prf: hand Windows the PRF input and let it apply the PRF hashing.
        // raw: hash it ourselves and tell Windows the salt is a raw hmac-secret value.
        byte[] salt = mode == "raw" ? PrfSalt(input) : input;
        uint flags = mode == "raw" ? Native.HMAC_SECRET_VALUES_FLAG : 0;
        Console.WriteLine("mode: " + mode);

        byte[] challenge = new byte[32];
        using (RandomNumberGenerator rng = RandomNumberGenerator.Create()) { rng.GetBytes(challenge); }
        byte[] json = Encoding.UTF8.GetBytes("{\"type\":\"webauthn.get\",\"challenge\":\"" + B64Url(challenge) +
                                             "\",\"origin\":\"https://localhost\",\"crossOrigin\":false}");

        ClientData cd = new ClientData();
        cd.dwVersion = 1;
        cd.cbClientDataJSON = (uint)json.Length;
        cd.pbClientDataJSON = Pin(json);
        cd.pwszHashAlgId = Marshal.StringToHGlobalUni("SHA-256");

        Credential cred = new Credential();
        cred.dwVersion = 1;
        cred.cbId = (uint)credId.Length;
        cred.pbId = Pin(credId);
        cred.pwszCredentialType = Marshal.StringToHGlobalUni("public-key");

        HmacSecretSalt hs = new HmacSecretSalt();
        hs.cbFirst = (uint)salt.Length;
        hs.pbFirst = Pin(salt);
        HmacSecretSaltValues hsv = new HmacSecretSaltValues();
        hsv.pGlobalHmacSalt = PinStruct(hs);

        GetAssertionOptions opts = new GetAssertionOptions();
        opts.dwVersion = 6;
        opts.dwTimeoutMilliseconds = 120000;
        opts.CredentialList.cCredentials = 1;
        opts.CredentialList.pCredentials = PinStruct(cred);
        opts.dwUserVerificationRequirement = 1; // required
        opts.dwFlags = flags;
        opts.pHmacSecretSaltValues = PinStruct(hsv);

        Console.WriteLine("Waiting for passkey. Choose your iPhone in the Windows dialog...");
        IntPtr pAssertion;
        int hr = Native.WebAuthNAuthenticatorGetAssertion(Native.GetForegroundWindow(), "localhost", ref cd, ref opts, out pAssertion);
        if (hr != 0) {
            Console.WriteLine("GetAssertion failed: 0x" + hr.ToString("X8") + " " + Marshal.PtrToStringUni(Native.WebAuthNGetErrorName(hr)));
            return 1;
        }
        try {
            Assertion a = (Assertion)Marshal.PtrToStructure(pAssertion, typeof(Assertion));
            Console.WriteLine("assertion version: " + a.dwVersion + ", transport: 0x" + a.dwUsedTransport.ToString("X"));
            if (a.dwVersion < 4 || a.pHmacSecret == IntPtr.Zero) {
                Console.WriteLine("NO PRF OUTPUT returned on the native path.");
                return 1;
            }
            HmacSecretSalt outSalt = (HmacSecretSalt)Marshal.PtrToStructure(a.pHmacSecret, typeof(HmacSecretSalt));
            byte[] output = new byte[outSalt.cbFirst];
            Marshal.Copy(outSalt.pbFirst, output, 0, output.Length);
            Console.WriteLine("PRF output: " + output.Length + " bytes, fingerprint " + Fingerprint(output));
            Console.WriteLine("Browser fingerprint was 95d41ca290954367. " +
                (Fingerprint(output) == "95d41ca290954367" ? "MATCH" : "DIFFERENT (try the other mode)"));
            return 0;
        } finally {
            Native.WebAuthNFreeAssertion(pAssertion);
        }
    }
}
