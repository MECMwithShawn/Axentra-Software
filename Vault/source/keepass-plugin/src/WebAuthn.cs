// Minimal interop with the Windows WebAuthn API (webauthn.dll): register a passkey with the
// PRF extension enabled, and evaluate PRF on an existing passkey. Struct layouts follow
// webauthn.h; only the fields up to the versions we request are declared.
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;

namespace AxentraVault
{
    internal sealed class WebAuthnException : Exception
    {
        public readonly string ErrorName;

        public WebAuthnException(string errorName, string message) : base(message)
        {
            ErrorName = errorName;
        }

        // The user dismissed the dialog or the request timed out.
        public bool IsCancellation
        {
            get { return ErrorName == "NotAllowedError" || ErrorName == "AbortError"; }
        }
    }

    internal static class WebAuthn
    {
        private const uint UvRequired = 1;
        private const uint AttestationNone = 1;
        private const uint TimeoutMs = 120000;

        [StructLayout(LayoutKind.Sequential)]
        private struct ClientData { public uint dwVersion; public uint cbClientDataJSON; public IntPtr pbClientDataJSON; public IntPtr pwszHashAlgId; }

        [StructLayout(LayoutKind.Sequential)]
        private struct RpEntity { public uint dwVersion; public IntPtr pwszId; public IntPtr pwszName; public IntPtr pwszIcon; }

        [StructLayout(LayoutKind.Sequential)]
        private struct UserEntity { public uint dwVersion; public uint cbId; public IntPtr pbId; public IntPtr pwszName; public IntPtr pwszIcon; public IntPtr pwszDisplayName; }

        [StructLayout(LayoutKind.Sequential)]
        private struct CoseParam { public uint dwVersion; public IntPtr pwszCredentialType; public int lAlg; }

        [StructLayout(LayoutKind.Sequential)]
        private struct CoseParams { public uint cCredentialParameters; public IntPtr pCredentialParameters; }

        [StructLayout(LayoutKind.Sequential)]
        private struct Credential { public uint dwVersion; public uint cbId; public IntPtr pbId; public IntPtr pwszCredentialType; }

        [StructLayout(LayoutKind.Sequential)]
        private struct Credentials { public uint cCredentials; public IntPtr pCredentials; }

        [StructLayout(LayoutKind.Sequential)]
        private struct Extensions { public uint cExtensions; public IntPtr pExtensions; }

        [StructLayout(LayoutKind.Sequential)]
        private struct Extension { public IntPtr pwszExtensionIdentifier; public uint cbExtension; public IntPtr pvExtension; }

        [StructLayout(LayoutKind.Sequential)]
        private struct HmacSecretSalt { public uint cbFirst; public IntPtr pbFirst; public uint cbSecond; public IntPtr pbSecond; }

        [StructLayout(LayoutKind.Sequential)]
        private struct CredWithSalt { public uint cbCredID; public IntPtr pbCredID; public IntPtr pHmacSecretSalt; }

        [StructLayout(LayoutKind.Sequential)]
        private struct HmacSecretSaltValues { public IntPtr pGlobalHmacSalt; public uint cCredWithHmacSecretSaltList; public IntPtr pCredWithHmacSecretSaltList; }

        // WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS through version 6.
        [StructLayout(LayoutKind.Sequential)]
        private struct MakeCredentialOptions
        {
            public uint dwVersion;
            public uint dwTimeoutMilliseconds;
            public Credentials CredentialList;
            public Extensions Extensions;
            public uint dwAuthenticatorAttachment;
            public int bRequireResidentKey;
            public uint dwUserVerificationRequirement;
            public uint dwAttestationConveyancePreference;
            public uint dwFlags;
            public IntPtr pCancellationId;          // v2
            public IntPtr pExcludeCredentialList;   // v3
            public uint dwEnterpriseAttestation;    // v4
            public uint dwLargeBlobSupport;
            public int bPreferResidentKey;
            public int bBrowserInPrivateMode;       // v5
            public int bEnablePrf;                  // v6
        }

        // WEBAUTHN_CREDENTIAL_ATTESTATION through version 5.
        [StructLayout(LayoutKind.Sequential)]
        private struct CredentialAttestation
        {
            public uint dwVersion;
            public IntPtr pwszFormatType;
            public uint cbAuthenticatorData;
            public IntPtr pbAuthenticatorData;
            public uint cbAttestation;
            public IntPtr pbAttestation;
            public uint dwAttestationDecodeType;
            public IntPtr pvAttestationDecode;
            public uint cbAttestationObject;
            public IntPtr pbAttestationObject;
            public uint cbCredentialId;
            public IntPtr pbCredentialId;
            public Extensions Extensions;           // v2
            public uint dwUsedTransport;            // v3
            public int bEpAtt;                      // v4
            public int bLargeBlobSupported;
            public int bResidentKey;
            public int bPrfEnabled;                 // v5
        }

        // WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS through version 6.
        [StructLayout(LayoutKind.Sequential)]
        private struct GetAssertionOptions
        {
            public uint dwVersion;
            public uint dwTimeoutMilliseconds;
            public Credentials CredentialList;
            public Extensions Extensions;
            public uint dwAuthenticatorAttachment;
            public uint dwUserVerificationRequirement;
            public uint dwFlags;
            public IntPtr pwszU2fAppId;             // v2
            public IntPtr pbU2fAppId;
            public IntPtr pCancellationId;          // v3
            public IntPtr pAllowCredentialList;     // v4
            public uint dwCredLargeBlobOperation;   // v5
            public uint cbCredLargeBlob;
            public IntPtr pbCredLargeBlob;
            public IntPtr pHmacSecretSaltValues;    // v6
            public int bBrowserInPrivateMode;
        }

        // WEBAUTHN_ASSERTION through version 4.
        [StructLayout(LayoutKind.Sequential)]
        private struct Assertion
        {
            public uint dwVersion;
            public uint cbAuthenticatorData;
            public IntPtr pbAuthenticatorData;
            public uint cbSignature;
            public IntPtr pbSignature;
            public Credential Credential;
            public uint cbUserId;
            public IntPtr pbUserId;
            public Extensions Extensions;           // v2
            public uint cbCredLargeBlob;            // v3
            public IntPtr pbCredLargeBlob;
            public uint dwCredLargeBlobStatus;
            public IntPtr pHmacSecret;              // v4
        }

        [DllImport("webauthn.dll")]
        private static extern uint WebAuthNGetApiVersionNumber();

        [DllImport("webauthn.dll", CharSet = CharSet.Unicode)]
        private static extern int WebAuthNAuthenticatorMakeCredential(IntPtr hWnd, IntPtr pRpInformation, IntPtr pUserInformation,
            IntPtr pPubKeyCredParams, IntPtr pClientData, IntPtr pOptions, out IntPtr ppAttestation);

        [DllImport("webauthn.dll", CharSet = CharSet.Unicode)]
        private static extern int WebAuthNAuthenticatorGetAssertion(IntPtr hWnd, string pwszRpId, IntPtr pClientData,
            IntPtr pOptions, out IntPtr ppAssertion);

        [DllImport("webauthn.dll")]
        private static extern void WebAuthNFreeCredentialAttestation(IntPtr pAttestation);

        [DllImport("webauthn.dll")]
        private static extern void WebAuthNFreeAssertion(IntPtr pAssertion);

        [DllImport("webauthn.dll")]
        private static extern IntPtr WebAuthNGetErrorName(int hr);

        public sealed class Registration
        {
            public byte[] CredentialId;
            public bool PrfEnabled;
        }

        // Tracks unmanaged allocations for one call so they can all be released in finally.
        private sealed class Arena : IDisposable
        {
            private readonly List<IntPtr> m_blocks = new List<IntPtr>();

            public IntPtr Bytes(byte[] data)
            {
                IntPtr p = Marshal.AllocHGlobal(data.Length);
                m_blocks.Add(p);
                Marshal.Copy(data, 0, p, data.Length);
                return p;
            }

            public IntPtr String(string s)
            {
                IntPtr p = Marshal.StringToHGlobalUni(s);
                m_blocks.Add(p);
                return p;
            }

            public IntPtr Alloc(int size)
            {
                IntPtr p = Marshal.AllocHGlobal(size);
                m_blocks.Add(p);
                return p;
            }

            public IntPtr Struct(object s)
            {
                IntPtr p = Marshal.AllocHGlobal(Marshal.SizeOf(s));
                m_blocks.Add(p);
                Marshal.StructureToPtr(s, p, false);
                return p;
            }

            public void Dispose()
            {
                foreach (IntPtr p in m_blocks) Marshal.FreeHGlobal(p);
                m_blocks.Clear();
            }
        }

        private static void RequireApi(uint minVersion)
        {
            uint v;
            try { v = WebAuthNGetApiVersionNumber(); }
            catch (EntryPointNotFoundException) { v = 0; }
            catch (DllNotFoundException) { v = 0; }
            if (v < minVersion)
                throw new WebAuthnException("NotSupportedError",
                    "This version of Windows does not support passkey PRF (WebAuthn API " + v + ", need " + minVersion + ").");
        }

        private static void Check(int hr, string what)
        {
            if (hr == 0) return;
            string name = Marshal.PtrToStringUni(WebAuthNGetErrorName(hr)) ?? "UnknownError";
            throw new WebAuthnException(name, what + " failed: " + name + " (0x" + hr.ToString("X8") + ").");
        }

        private static IntPtr ClientDataFor(Arena arena, string type, string rpId)
        {
            // There is no relying-party server: the challenge only has to be fresh.
            byte[] challenge = Random(32);
            string json = "{\"type\":\"" + type + "\",\"challenge\":\"" + Base64Url(challenge) +
                          "\",\"origin\":\"https://" + rpId + "\",\"crossOrigin\":false}";
            byte[] jsonBytes = Encoding.UTF8.GetBytes(json);

            ClientData cd = new ClientData();
            cd.dwVersion = 1;
            cd.cbClientDataJSON = (uint)jsonBytes.Length;
            cd.pbClientDataJSON = arena.Bytes(jsonBytes);
            cd.pwszHashAlgId = arena.String("SHA-256");
            return arena.Struct(cd);
        }

        public static Registration Register(IntPtr hWnd, string rpId, string rpName, byte[] userId, string userName, string displayName)
        {
            RequireApi(6);
            using (Arena arena = new Arena())
            {
                RpEntity rp = new RpEntity();
                rp.dwVersion = 1;
                rp.pwszId = arena.String(rpId);
                rp.pwszName = arena.String(rpName);

                UserEntity user = new UserEntity();
                user.dwVersion = 1;
                user.cbId = (uint)userId.Length;
                user.pbId = arena.Bytes(userId);
                user.pwszName = arena.String(userName);
                user.pwszDisplayName = arena.String(displayName);

                // ES256 first, RS256 as fallback.
                IntPtr type = arena.String("public-key");
                int size = Marshal.SizeOf(typeof(CoseParam));
                IntPtr paramArray = Marshal.AllocHGlobal(size * 2);
                try
                {
                    CoseParam es256 = new CoseParam(); es256.dwVersion = 1; es256.pwszCredentialType = type; es256.lAlg = -7;
                    CoseParam rs256 = new CoseParam(); rs256.dwVersion = 1; rs256.pwszCredentialType = type; rs256.lAlg = -257;
                    Marshal.StructureToPtr(es256, paramArray, false);
                    Marshal.StructureToPtr(rs256, new IntPtr(paramArray.ToInt64() + size), false);

                    CoseParams cose = new CoseParams();
                    cose.cCredentialParameters = 2;
                    cose.pCredentialParameters = paramArray;

                    MakeCredentialOptions opts = new MakeCredentialOptions();
                    opts.dwVersion = 6;
                    opts.dwTimeoutMilliseconds = TimeoutMs;
                    opts.bRequireResidentKey = 1;
                    opts.bPreferResidentKey = 1;
                    opts.dwUserVerificationRequirement = UvRequired;
                    opts.dwAttestationConveyancePreference = AttestationNone;
                    opts.bEnablePrf = 1;
                    // CTAP security keys (YubiKey) only create the per-credential PRF secret when
                    // hmac-secret is requested at registration; bEnablePrf alone covers phones.
                    Extension hmac = new Extension();
                    hmac.pwszExtensionIdentifier = arena.String("hmac-secret");
                    hmac.cbExtension = 4;
                    hmac.pvExtension = arena.Bytes(BitConverter.GetBytes(1));
                    opts.Extensions.cExtensions = 1;
                    opts.Extensions.pExtensions = arena.Struct(hmac);

                    IntPtr pAtt;
                    int hr = WebAuthNAuthenticatorMakeCredential(hWnd, arena.Struct(rp), arena.Struct(user), arena.Struct(cose),
                        ClientDataFor(arena, "webauthn.create", rpId), arena.Struct(opts), out pAtt);
                    Check(hr, "Passkey registration");
                    try
                    {
                        CredentialAttestation att = (CredentialAttestation)Marshal.PtrToStructure(pAtt, typeof(CredentialAttestation));
                        Registration reg = new Registration();
                        reg.CredentialId = new byte[att.cbCredentialId];
                        Marshal.Copy(att.pbCredentialId, reg.CredentialId, 0, reg.CredentialId.Length);
                        // Older attestation structs do not report PRF; the first evaluation is the real test.
                        reg.PrfEnabled = att.dwVersion < 5 || att.bPrfEnabled != 0;
                        return reg;
                    }
                    finally { WebAuthNFreeCredentialAttestation(pAtt); }
                }
                finally { Marshal.FreeHGlobal(paramArray); }
            }
        }

        // Returns the 32-byte PRF output for prfInput. Windows applies the WebAuthn PRF
        // salt hashing itself, so results match a browser's prf.eval.first.
        public static byte[] EvaluatePrf(IntPtr hWnd, string rpId, byte[] credentialId, byte[] prfInput)
        {
            int used;
            return EvaluatePrfAny(hWnd, rpId, new byte[][] { credentialId }, new byte[][] { prfInput }, out used);
        }

        // Offers every given passkey (phone, security key, ...) in one Windows dialog and evaluates
        // PRF on the one the user picks; `used` is its index.
        public static byte[] EvaluatePrfAny(IntPtr hWnd, string rpId, IList<byte[]> credentialIds, IList<byte[]> prfInputs, out int used)
        {
            RequireApi(6);
            int n = credentialIds.Count;
            if (n == 0 || prfInputs.Count != n) throw new ArgumentException("No passkeys to ask.");
            using (Arena arena = new Arena())
            {
                IntPtr type = arena.String("public-key");
                int credSize = Marshal.SizeOf(typeof(Credential));
                IntPtr creds = arena.Alloc(credSize * n);
                bool shared = true;
                for (int i = 0; i < n; i++)
                {
                    Credential cred = new Credential();
                    cred.dwVersion = 1;
                    cred.cbId = (uint)credentialIds[i].Length;
                    cred.pbId = arena.Bytes(credentialIds[i]);
                    cred.pwszCredentialType = type;
                    Marshal.StructureToPtr(cred, IntPtr.Add(creds, i * credSize), false);
                    if (!SameBytes(prfInputs[i], prfInputs[0])) shared = false;
                }

                // One shared PRF input goes in the global salt; differing inputs go per credential.
                HmacSecretSaltValues saltValues = new HmacSecretSaltValues();
                if (shared)
                {
                    saltValues.pGlobalHmacSalt = arena.Struct(SaltFor(arena, prfInputs[0]));
                }
                else
                {
                    int size = Marshal.SizeOf(typeof(CredWithSalt));
                    IntPtr list = arena.Alloc(size * n);
                    for (int i = 0; i < n; i++)
                    {
                        CredWithSalt c = new CredWithSalt();
                        c.cbCredID = (uint)credentialIds[i].Length;
                        c.pbCredID = arena.Bytes(credentialIds[i]);
                        c.pHmacSecretSalt = arena.Struct(SaltFor(arena, prfInputs[i]));
                        Marshal.StructureToPtr(c, IntPtr.Add(list, i * size), false);
                    }
                    saltValues.cCredWithHmacSecretSaltList = (uint)n;
                    saltValues.pCredWithHmacSecretSaltList = list;
                }

                GetAssertionOptions opts = new GetAssertionOptions();
                opts.dwVersion = 6;
                opts.dwTimeoutMilliseconds = TimeoutMs;
                opts.CredentialList.cCredentials = (uint)n;
                opts.CredentialList.pCredentials = creds;
                opts.dwUserVerificationRequirement = UvRequired;
                opts.pHmacSecretSaltValues = arena.Struct(saltValues);

                IntPtr pAssertion;
                int hr = WebAuthNAuthenticatorGetAssertion(hWnd, rpId, ClientDataFor(arena, "webauthn.get", rpId),
                    arena.Struct(opts), out pAssertion);
                Check(hr, "Passkey unlock");
                try
                {
                    Assertion a = (Assertion)Marshal.PtrToStructure(pAssertion, typeof(Assertion));
                    used = 0;
                    if (n > 1)
                    {
                        byte[] id = new byte[a.Credential.cbId];
                        Marshal.Copy(a.Credential.pbId, id, 0, id.Length);
                        used = -1;
                        for (int i = 0; i < n; i++) if (SameBytes(id, credentialIds[i])) used = i;
                        if (used < 0)
                            throw new WebAuthnException("NotAllowedError", "Windows answered with a passkey that is not registered for this database.");
                    }
                    if (a.dwVersion < 4 || a.pHmacSecret == IntPtr.Zero)
                        throw new WebAuthnException("NotSupportedError", "The passkey did not return a PRF secret.");
                    HmacSecretSalt result = (HmacSecretSalt)Marshal.PtrToStructure(a.pHmacSecret, typeof(HmacSecretSalt));
                    if (result.cbFirst != 32)
                        throw new WebAuthnException("NotSupportedError", "Unexpected PRF output length " + result.cbFirst + ".");
                    byte[] output = new byte[32];
                    Marshal.Copy(result.pbFirst, output, 0, 32);
                    return output;
                }
                finally { WebAuthNFreeAssertion(pAssertion); }
            }
        }

        private static HmacSecretSalt SaltFor(Arena arena, byte[] prfInput)
        {
            HmacSecretSalt salt = new HmacSecretSalt();
            salt.cbFirst = (uint)prfInput.Length;
            salt.pbFirst = arena.Bytes(prfInput);
            return salt;
        }

        private static bool SameBytes(byte[] a, byte[] b)
        {
            if (a.Length != b.Length) return false;
            for (int i = 0; i < a.Length; i++) if (a[i] != b[i]) return false;
            return true;
        }

        internal static byte[] Random(int n)
        {
            byte[] b = new byte[n];
            using (RandomNumberGenerator rng = RandomNumberGenerator.Create()) { rng.GetBytes(b); }
            return b;
        }

        private static string Base64Url(byte[] b)
        {
            return Convert.ToBase64String(b).TrimEnd('=').Replace('+', '-').Replace('/', '_');
        }
    }
}
