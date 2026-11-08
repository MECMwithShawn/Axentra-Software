// The sidecar file (<database>.axvault) that stores the database key wrapped under the
// passkey's PRF secret. It holds no usable secret on its own: unwrapping needs the passkey.
using System;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Text;

namespace AxentraVault
{
    internal sealed class Envelope
    {
        public const string Magic = "AxentraVault/1";
        // Several passkeys / security keys, each in a "[key]" block, all wrapping the same key.
        public const string MagicV2 = "AxentraVault/2";
        public const string Extension = ".axvault";
        private const string HkdfInfo = "axentra-vault/v1 aes-256-cbc+hmac-sha256";

        public string RpId;
        public byte[] CredentialId;
        public byte[] PrfInput;
        public byte[] Iv;
        public byte[] Ciphertext;
        public byte[] Mac;

        public static string PathFor(string databasePath)
        {
            return databasePath + Extension;
        }

        // Wraps key with AES-256-CBC, then HMAC-SHA256 over every other field (encrypt-then-MAC).
        // Keys come from HKDF-SHA256(prfOutput, salt = PrfInput).
        public static Envelope Seal(string rpId, byte[] credentialId, byte[] prfInput, byte[] prfOutput, byte[] key)
        {
            Envelope e = new Envelope();
            e.RpId = rpId;
            e.CredentialId = credentialId;
            e.PrfInput = prfInput;
            e.Iv = WebAuthn.Random(16);

            byte[] encKey, macKey;
            e.DeriveKeys(prfOutput, out encKey, out macKey);
            try
            {
                using (Aes aes = Aes.Create())
                {
                    aes.Mode = CipherMode.CBC;
                    aes.Padding = PaddingMode.PKCS7;
                    using (ICryptoTransform t = aes.CreateEncryptor(encKey, e.Iv))
                        e.Ciphertext = t.TransformFinalBlock(key, 0, key.Length);
                }
                e.Mac = e.ComputeMac(macKey);
                return e;
            }
            finally { Wipe(encKey); Wipe(macKey); }
        }

        // Throws CryptographicException if prfOutput is wrong or any field was modified.
        public byte[] Open(byte[] prfOutput)
        {
            byte[] encKey, macKey;
            DeriveKeys(prfOutput, out encKey, out macKey);
            try
            {
                byte[] expected = ComputeMac(macKey);
                if (!FixedTimeEquals(expected, Mac))
                    throw new CryptographicException("The passkey secret does not match this database's key envelope.");
                using (Aes aes = Aes.Create())
                {
                    aes.Mode = CipherMode.CBC;
                    aes.Padding = PaddingMode.PKCS7;
                    using (ICryptoTransform t = aes.CreateDecryptor(encKey, Iv))
                        return t.TransformFinalBlock(Ciphertext, 0, Ciphertext.Length);
                }
            }
            finally { Wipe(encKey); Wipe(macKey); }
        }

        public string Serialize()
        {
            StringBuilder sb = new StringBuilder();
            sb.Append(Magic).Append('\n');
            sb.Append("rpId=").Append(RpId).Append('\n');
            sb.Append("credentialId=").Append(Convert.ToBase64String(CredentialId)).Append('\n');
            sb.Append("prfInput=").Append(Convert.ToBase64String(PrfInput)).Append('\n');
            sb.Append("iv=").Append(Convert.ToBase64String(Iv)).Append('\n');
            sb.Append("ciphertext=").Append(Convert.ToBase64String(Ciphertext)).Append('\n');
            sb.Append("mac=").Append(Convert.ToBase64String(Mac)).Append('\n');
            return sb.ToString();
        }

        public static Envelope Parse(string text)
        {
            string[] lines = text.Replace("\r", "").Split(new char[] { '\n' }, StringSplitOptions.RemoveEmptyEntries);
            if (lines.Length == 0 || lines[0].Trim() != Magic)
                throw new FormatException("Not an Axentra Vault file (expected " + Magic + ").");

            Dictionary<string, string> f = new Dictionary<string, string>();
            for (int i = 1; i < lines.Length; i++)
            {
                int eq = lines[i].IndexOf('=');
                if (eq > 0) f[lines[i].Substring(0, eq).Trim()] = lines[i].Substring(eq + 1).Trim();
            }

            Envelope e = new Envelope();
            e.RpId = Field(f, "rpId");
            e.CredentialId = Convert.FromBase64String(Field(f, "credentialId"));
            e.PrfInput = Convert.FromBase64String(Field(f, "prfInput"));
            e.Iv = Convert.FromBase64String(Field(f, "iv"));
            e.Ciphertext = Convert.FromBase64String(Field(f, "ciphertext"));
            e.Mac = Convert.FromBase64String(Field(f, "mac"));
            if (e.Iv.Length != 16 || e.Mac.Length != 32 || e.PrfInput.Length != 32)
                throw new FormatException("Axentra Vault file has invalid field lengths.");
            return e;
        }

        // Every key in the file: the one key of an AxentraVault/1 file, or each [key] block of an
        // AxentraVault/2 file (written by the Axentra Vault app when a second key is added).
        public static List<Envelope> ParseAll(string text)
        {
            string t = text.Replace("\r", "");
            string first = null;
            foreach (string line in t.Split('\n'))
            {
                if (line.Trim().Length > 0) { first = line.Trim(); break; }
            }
            List<Envelope> all = new List<Envelope>();
            if (first == MagicV2)
            {
                string[] blocks = t.Split(new string[] { "[key]" }, StringSplitOptions.None);
                for (int i = 1; i < blocks.Length; i++) all.Add(Parse(Magic + "\n" + blocks[i]));
                if (all.Count == 0) throw new FormatException("Axentra Vault file lists no keys.");
            }
            else
            {
                all.Add(Parse(text));
            }
            return all;
        }

        private static string Field(Dictionary<string, string> f, string name)
        {
            string v;
            if (!f.TryGetValue(name, out v) || v.Length == 0)
                throw new FormatException("Axentra Vault file is missing '" + name + "'.");
            return v;
        }

        private void DeriveKeys(byte[] prfOutput, out byte[] encKey, out byte[] macKey)
        {
            byte[] okm = Hkdf(prfOutput, PrfInput, Encoding.ASCII.GetBytes(HkdfInfo), 64);
            encKey = new byte[32];
            macKey = new byte[32];
            Buffer.BlockCopy(okm, 0, encKey, 0, 32);
            Buffer.BlockCopy(okm, 32, macKey, 0, 32);
            Wipe(okm);
        }

        private byte[] ComputeMac(byte[] macKey)
        {
            // Length-prefix every field so no two field sets serialize to the same bytes.
            List<byte> data = new List<byte>();
            AppendField(data, Encoding.ASCII.GetBytes(Magic));
            AppendField(data, Encoding.UTF8.GetBytes(RpId));
            AppendField(data, CredentialId);
            AppendField(data, PrfInput);
            AppendField(data, Iv);
            AppendField(data, Ciphertext);
            using (HMACSHA256 h = new HMACSHA256(macKey)) { return h.ComputeHash(data.ToArray()); }
        }

        private static void AppendField(List<byte> data, byte[] field)
        {
            data.AddRange(BitConverter.GetBytes(field.Length));
            data.AddRange(field);
        }

        // RFC 5869 HKDF-SHA256.
        internal static byte[] Hkdf(byte[] ikm, byte[] salt, byte[] info, int length)
        {
            byte[] prk;
            using (HMACSHA256 h = new HMACSHA256(salt)) { prk = h.ComputeHash(ikm); }
            byte[] okm = new byte[length];
            byte[] t = new byte[0];
            int pos = 0;
            for (byte i = 1; pos < length; i++)
            {
                byte[] input = new byte[t.Length + info.Length + 1];
                Buffer.BlockCopy(t, 0, input, 0, t.Length);
                Buffer.BlockCopy(info, 0, input, t.Length, info.Length);
                input[input.Length - 1] = i;
                using (HMACSHA256 h = new HMACSHA256(prk)) { t = h.ComputeHash(input); }
                int n = Math.Min(t.Length, length - pos);
                Buffer.BlockCopy(t, 0, okm, pos, n);
                pos += n;
            }
            Wipe(prk);
            Wipe(t);
            return okm;
        }

        private static bool FixedTimeEquals(byte[] a, byte[] b)
        {
            if (a == null || b == null || a.Length != b.Length) return false;
            int diff = 0;
            for (int i = 0; i < a.Length; i++) diff |= a[i] ^ b[i];
            return diff == 0;
        }

        internal static void Wipe(byte[] b)
        {
            if (b != null) Array.Clear(b, 0, b.Length);
        }
    }
}
