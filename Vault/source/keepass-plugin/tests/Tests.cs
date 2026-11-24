// Offline tests (no passkey needed): crypto envelope, key file format, and that a database
// keyed through the provider opens with the backup key file in stock KeePassLib.
using System;
using System.Collections.Generic;
using System.IO;
using System.Security.Cryptography;
using System.Text;

using KeePassLib;
using KeePassLib.Keys;
using KeePassLib.Serialization;

namespace AxentraVault
{
    internal static class Tests
    {
        private static int s_failed;
        private static string s_tmp;

        private static int Main()
        {
            s_tmp = Path.Combine(Path.GetTempPath(), "axvault-tests-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(s_tmp);
            try
            {
                Run("HKDF matches RFC 5869 test case 1", HkdfVector);
                Run("Envelope seal/open round trip through serialization", EnvelopeRoundTrip);
                Run("Envelope rejects a different PRF secret", EnvelopeWrongSecret);
                Run("Envelope rejects tampering with any field", EnvelopeTamper);
                Run("Envelope parser rejects foreign files", EnvelopeParseErrors);
                Run("Multi-key file: every key opens the same database key", KeyRingParse);
                Run("Backup key file loads as the same key in KeePassLib", KeyFileLoads);
                Run("Provider-keyed database opens with password + backup key file", DatabaseEquivalence);
                Run("Database rejects password + wrong key file", DatabaseRejectsWrongKey);
            }
            finally
            {
                try { Directory.Delete(s_tmp, true); } catch (IOException) { }
            }
            Console.WriteLine(s_failed == 0 ? "\nALL PASSED" : "\n" + s_failed + " FAILED");
            return s_failed == 0 ? 0 : 1;
        }

        private static void Run(string name, Action test)
        {
            try { test(); Console.WriteLine("PASS  " + name); }
            catch (Exception ex) { s_failed++; Console.WriteLine("FAIL  " + name + "\n      " + ex.GetType().Name + ": " + ex.Message); }
        }

        private static void Assert(bool cond, string message)
        {
            if (!cond) throw new Exception(message);
        }

        private static bool Same(byte[] a, byte[] b)
        {
            if (a.Length != b.Length) return false;
            for (int i = 0; i < a.Length; i++) if (a[i] != b[i]) return false;
            return true;
        }

        private static byte[] Hex(string s)
        {
            byte[] b = new byte[s.Length / 2];
            for (int i = 0; i < b.Length; i++) b[i] = Convert.ToByte(s.Substring(i * 2, 2), 16);
            return b;
        }

        private static void HkdfVector()
        {
            byte[] ikm = Hex("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b");
            byte[] salt = Hex("000102030405060708090a0b0c");
            byte[] info = Hex("f0f1f2f3f4f5f6f7f8f9");
            byte[] okm = Envelope.Hkdf(ikm, salt, info, 42);
            Assert(Same(okm, Hex("3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865")), "OKM mismatch");
        }

        private static Envelope Sample(out byte[] prf, out byte[] key)
        {
            prf = WebAuthn.Random(32);
            key = WebAuthn.Random(32);
            return Envelope.Seal("localhost", WebAuthn.Random(20), WebAuthn.Random(32), prf, key);
        }

        private static void EnvelopeRoundTrip()
        {
            byte[] prf, key;
            Envelope e = Sample(out prf, out key);
            Envelope parsed = Envelope.Parse(e.Serialize());
            Assert(Same(parsed.Open(prf), key), "unwrapped key differs");
            Assert(parsed.RpId == "localhost", "rpId lost");
        }

        private static void EnvelopeWrongSecret()
        {
            byte[] prf, key;
            Envelope e = Sample(out prf, out key);
            ExpectCryptoFailure(e, WebAuthn.Random(32), "wrong secret accepted");
        }

        private static void EnvelopeTamper()
        {
            byte[] prf, key;
            string text = Sample(out prf, out key).Serialize();
            foreach (string field in new string[] { "CredentialId", "PrfInput", "Iv", "Ciphertext", "Mac" })
            {
                Envelope e = Envelope.Parse(text);
                byte[] target = (byte[])typeof(Envelope).GetField(field).GetValue(e);
                target[0] ^= 0x01;
                ExpectCryptoFailure(e, prf, field + " tamper accepted");
            }
            Envelope r = Envelope.Parse(text);
            r.RpId = "example.com";
            ExpectCryptoFailure(r, prf, "RpId tamper accepted");
        }

        private static void ExpectCryptoFailure(Envelope e, byte[] prf, string message)
        {
            try { e.Open(prf); }
            catch (CryptographicException) { return; }
            throw new Exception(message);
        }

        private static void KeyRingParse()
        {
            byte[] prfA, key;
            Envelope a = Sample(out prfA, out key);
            byte[] prfB = WebAuthn.Random(32);
            Envelope b = Envelope.Seal("localhost", new byte[] { 9, 9 }, a.PrfInput, prfB, key);
            Func<Envelope, string, string> block = (e, label) =>
                "\n[key]\nlabel=" + label + "\n" + e.Serialize().Substring(Envelope.Magic.Length + 1);
            string text = Envelope.MagicV2 + "\n" + block(a, "Phone passkey") + block(b, "Security key (USB)");
            List<Envelope> all = Envelope.ParseAll(text.Replace("\n", "\r\n"));
            Assert(all.Count == 2, "expected two keys, got " + all.Count);
            Assert(Same(all[0].Open(prfA), key), "first key unwrapped a different key");
            Assert(Same(all[1].Open(prfB), key), "second key unwrapped a different key");
            Assert(Envelope.ParseAll(a.Serialize()).Count == 1, "v1 file is one key");
            Assert(Envelope.ParseAll(Envelope.Magic + "\nlabel=Phone passkey\n" + a.Serialize().Substring(Envelope.Magic.Length + 1)).Count == 1,
                "v1 file with a label line still parses");
        }

        private static void EnvelopeParseErrors()
        {
            foreach (string bad in new string[] { "", "something else\nrpId=x", Envelope.Magic + "\nrpId=localhost" })
            {
                try { Envelope.Parse(bad); }
                catch (FormatException) { continue; }
                throw new Exception("parsed invalid input: " + bad);
            }
        }

        private static void KeyFileLoads()
        {
            byte[] key = WebAuthn.Random(32);
            string path = Path.Combine(s_tmp, "load.keyx");
            KeyFileWriter.Write(path, key);
            KcpKeyFile kf = new KcpKeyFile(path);
            Assert(Same(kf.KeyData.ReadData(), key), "KeePassLib read different key bytes from the .keyx");
        }

        // Mirrors how KeePass builds the composite key from a provider: KcpCustomKey with
        // hashing disabled because DirectKey is true.
        private static CompositeKey ProviderKey(string password, byte[] key)
        {
            PasskeyKeyProvider p = new PasskeyKeyProvider();
            CompositeKey ck = new CompositeKey();
            ck.AddUserKey(new KcpPassword(password));
            ck.AddUserKey(new KcpCustomKey(p.Name, key, !p.DirectKey));
            return ck;
        }

        private static string CreateDatabase(string name, string password, byte[] key)
        {
            string path = Path.Combine(s_tmp, name);
            PwDatabase db = new PwDatabase();
            db.New(IOConnectionInfo.FromPath(path), ProviderKey(password, key));
            db.Name = "passkey test";
            db.Save(null);
            db.Close();
            return path;
        }

        private static void DatabaseEquivalence()
        {
            Assert(new PasskeyKeyProvider().DirectKey, "provider must use DirectKey for key-file equivalence");
            byte[] key = WebAuthn.Random(32);
            string dbPath = CreateDatabase("equiv.kdbx", "correct horse", key);
            string kfPath = Path.Combine(s_tmp, "equiv.keyx");
            KeyFileWriter.Write(kfPath, key);

            CompositeKey ck = new CompositeKey();
            ck.AddUserKey(new KcpPassword("correct horse"));
            ck.AddUserKey(new KcpKeyFile(kfPath));
            PwDatabase db = new PwDatabase();
            db.Open(IOConnectionInfo.FromPath(dbPath), ck, null);
            Assert(db.Name == "passkey test", "opened database has wrong contents");
            db.Close();
        }

        private static void DatabaseRejectsWrongKey()
        {
            string dbPath = CreateDatabase("wrong.kdbx", "correct horse", WebAuthn.Random(32));
            string kfPath = Path.Combine(s_tmp, "wrong.keyx");
            KeyFileWriter.Write(kfPath, WebAuthn.Random(32));

            CompositeKey ck = new CompositeKey();
            ck.AddUserKey(new KcpPassword("correct horse"));
            ck.AddUserKey(new KcpKeyFile(kfPath));
            try { new PwDatabase().Open(IOConnectionInfo.FromPath(dbPath), ck, null); }
            catch (InvalidCompositeKeyException) { return; }
            throw new Exception("database opened with the wrong key file");
        }
    }
}
