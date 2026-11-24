// Interop fixtures between KeePass (KeePassLib) and the Rust app.
//   Interop.exe make <dir>    write fixture.kdbx (every KeePass feature, Argon2id) and csharp.axvault
//   Interop.exe verify <dir>  check rust-resaved.kdbx in KeePassLib (and that KeePass keeps its
//                             credential record), and that rust.axvault opens here
// Build: see run.ps1 (compiled together with keepass-plugin/src for the Envelope class).
using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Security.Cryptography;
using System.Text;

using KeePassLib;
using KeePassLib.Collections;
using KeePassLib.Cryptography.KeyDerivation;
using KeePassLib.Keys;
using KeePassLib.Security;
using KeePassLib.Serialization;
using KeePassLib.Utility;

namespace AxentraVault
{
    internal static class Interop
    {
        internal const string Password = "fixture-password";
        internal static readonly byte[] KeyFileBytes = Fill(0x5A, 32);
        internal static readonly byte[] PrfOutput = Fill(0x11, 32);
        internal static readonly byte[] EnvelopeKey = Fill(0x22, 32);
        internal static readonly byte[] PrfInput = Fill(0x33, 32);
        internal static readonly byte[] CredentialId = new byte[] { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16 };

        // Smallest valid PNG (1x1 transparent), for the custom icon.
        private static readonly byte[] Png = Convert.FromBase64String(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==");

        private static int Main(string[] args)
        {
            if (args.Length != 2) { Console.WriteLine("usage: Interop.exe make|verify <dir>"); return 2; }
            try
            {
                if (args[0] == "make") return Make(args[1]);
                if (args[0] == "verify") return Verify(args[1]);
            }
            catch (Exception ex) { Console.WriteLine("ERROR " + ex); return 1; }
            return 2;
        }

        private static byte[] Fill(byte b, int n)
        {
            byte[] a = new byte[n];
            for (int i = 0; i < n; i++) a[i] = b;
            return a;
        }

        internal static CompositeKey Key()
        {
            CompositeKey ck = new CompositeKey();
            ck.AddUserKey(new KcpPassword(Password));
            ck.AddUserKey(new KcpCustomKey("Axentra Vault", KeyFileBytes, false));
            return ck;
        }

        private static int Make(string dir)
        {
            Directory.CreateDirectory(dir);
            string path = Path.Combine(dir, "fixture.kdbx");
            PwDatabase db = new PwDatabase();
            db.New(IOConnectionInfo.FromPath(path), Key());

            // The user's real settings: AES-256 outer cipher, Argon2id. Memory reduced to 16 MB
            // so the test runs quickly; the format is identical.
            Argon2Kdf kdf = new Argon2Kdf(Argon2Type.ID);
            KdfParameters p = kdf.GetDefaultParameters();
            p.SetUInt64(Argon2Kdf.ParamIterations, 6);
            p.SetUInt64(Argon2Kdf.ParamMemory, 16UL * 1024 * 1024);
            p.SetUInt32(Argon2Kdf.ParamParallelism, 2);
            db.KdfParameters = p;

            db.Name = "Fixture & <Test>";
            db.Description = "All KeePass features";
            db.DefaultUserName = "shawn";
            db.Color = Color.FromArgb(0x33, 0x66, 0x99);
            db.MaintenanceHistoryDays = 180;
            db.HistoryMaxItems = 7;
            db.CustomData.Set("meta-key", "meta-value");
            db.PublicCustomData.SetString("public-key", "public-value");

            PwUuid iconId = new PwUuid(true);
            db.CustomIcons.Add(new PwCustomIcon(iconId, Png));

            PwGroup banking = new PwGroup(true, true, "Banking & Finance", PwIcon.Money);
            banking.Notes = "Group notes";
            banking.CustomData.Set("group-key", "group-value");
            banking.DefaultAutoTypeSequence = "{USERNAME}{TAB}{PASSWORD}";
            banking.EnableSearching = false;
            banking.Tags.Add("money");
            db.RootGroup.AddGroup(banking, true);

            PwGroup nested = new PwGroup(true, true, "Nested", PwIcon.Folder);
            nested.CustomIconUuid = iconId;
            banking.AddGroup(nested, true);

            PwEntry e = new PwEntry(true, true);
            e.Strings.Set(PwDefs.TitleField, new ProtectedString(false, "Bank <main> & \"co\""));
            e.Strings.Set(PwDefs.UserNameField, new ProtectedString(false, "shawn"));
            e.Strings.Set(PwDefs.PasswordField, new ProtectedString(true, "old-password"));
            e.Strings.Set(PwDefs.UrlField, new ProtectedString(false, "https://bank.example/login?a=1&b=2"));
            e.Strings.Set(PwDefs.NotesField, new ProtectedString(false, "line 1\r\nline 2 é中\U0001F511"));
            e.Strings.Set("PIN", new ProtectedString(true, "1234"));
            e.Strings.Set("Account", new ProtectedString(false, "000-111"));
            e.Strings.Set("otp", new ProtectedString(true, "otpauth://totp/x?secret=JBSWY3DPEHPK3PXP"));
            e.Binaries.Set("statement.txt", new ProtectedBinary(false, Encoding.UTF8.GetBytes("hello attachment")));
            e.Binaries.Set("secret.bin", new ProtectedBinary(true, Fill(0x7F, 300)));
            e.AutoType.DefaultSequence = "{USERNAME}{TAB}{PASSWORD}{ENTER}";
            e.AutoType.Add(new AutoTypeAssociation("Bank - *", "{PASSWORD}{ENTER}"));
            e.ForegroundColor = Color.FromArgb(0xC0, 0x10, 0x20);
            e.BackgroundColor = Color.FromArgb(0xF0, 0xF0, 0xE0);
            e.OverrideUrl = "cmd://\"{URL}\"";
            e.QualityCheck = false;
            e.Tags.Add("money");
            e.Tags.Add("important");
            e.CustomData.Set("entry-key", "entry-value");
            e.Expires = true;
            e.ExpiryTime = new DateTime(2030, 1, 2, 3, 4, 5, DateTimeKind.Utc);
            e.CustomIconUuid = iconId;
            banking.AddEntry(e, true);

            // Two history versions.
            e.CreateBackup(db);
            e.Strings.Set(PwDefs.PasswordField, new ProtectedString(true, "middle-password"));
            e.CreateBackup(db);
            e.Strings.Set(PwDefs.PasswordField, new ProtectedString(true, "current-password"));

            PwEntry plain = new PwEntry(true, true);
            plain.Strings.Set(PwDefs.TitleField, new ProtectedString(false, "Email"));
            plain.Strings.Set(PwDefs.PasswordField, new ProtectedString(true, "e-mail-pw"));
            nested.AddEntry(plain, true);

            // Recycle bin with a deleted entry, plus a deleted-object record.
            PwGroup bin = new PwGroup(true, true, "Recycle Bin", PwIcon.TrashBin);
            bin.EnableAutoType = false;
            bin.EnableSearching = false;
            db.RootGroup.AddGroup(bin, true);
            db.RecycleBinEnabled = true;
            db.RecycleBinUuid = bin.Uuid;
            PwEntry trashed = new PwEntry(true, true);
            trashed.Strings.Set(PwDefs.TitleField, new ProtectedString(false, "Old account"));
            bin.AddEntry(trashed, true);
            db.DeletedObjects.Add(new PwDeletedObject(new PwUuid(true), new DateTime(2025, 5, 6, 7, 8, 9, DateTimeKind.Utc)));

            db.Save(null);
            db.Close();
            Console.WriteLine("wrote " + path);

            Envelope env = Envelope.Seal("localhost", CredentialId, PrfInput, PrfOutput, EnvelopeKey);
            File.WriteAllText(Path.Combine(dir, "csharp.axvault"), env.Serialize(), new UTF8Encoding(false));
            Console.WriteLine("wrote csharp.axvault");
            return 0;
        }

        private const string CredentialRecordKey = "AxentraVault.Credentials";

        // "v1;<generation>;<salt>;<check>" with check = HMAC-SHA256(salt, label || composite),
        // the composite computed the way KeePassLib does: SHA-256 over every user key's data.
        private static bool CredentialCheckMatches(string record, CompositeKey key)
        {
            if (record == null) return false;
            string[] parts = record.Split(';');
            if (parts.Length != 4 || parts[0] != "v1" || parts[1].Length != 32 ||
                parts[2].Length != 64 || parts[3].Length != 64) return false;
            byte[] salt = MemUtil.HexStringToByteArray(parts[2]);
            byte[] check = MemUtil.HexStringToByteArray(parts[3]);
            byte[] composite;
            using (MemoryStream ms = new MemoryStream())
            {
                foreach (IUserKey uk in key.UserKeys)
                {
                    byte[] data = uk.KeyData.ReadData();
                    ms.Write(data, 0, data.Length);
                }
                using (SHA256 sha = SHA256.Create()) composite = sha.ComputeHash(ms.ToArray());
            }
            byte[] label = Encoding.ASCII.GetBytes("axentra-vault/credential-check/v1");
            byte[] message = new byte[label.Length + composite.Length];
            Array.Copy(label, message, label.Length);
            Array.Copy(composite, 0, message, label.Length, composite.Length);
            using (HMACSHA256 hmac = new HMACSHA256(salt))
                return MemUtil.ArraysEqual(hmac.ComputeHash(message), check);
        }

        private static int failures;

        private static void Check(bool ok, string what)
        {
            Console.WriteLine((ok ? "PASS  " : "FAIL  ") + what);
            if (!ok) failures++;
        }

        private static int Verify(string dir)
        {
            PwDatabase db = new PwDatabase();
            db.Open(IOConnectionInfo.FromPath(Path.Combine(dir, "rust-resaved.kdbx")), Key(), null);

            Check(db.Name == "Fixture & <Test>", "database name");
            Check(db.CustomData.Get("meta-key") == "meta-value", "database custom data");
            string record = db.CustomData.Get(CredentialRecordKey);
            Check(CredentialCheckMatches(record, Key()), "Axentra credential record checks KeePass's own composite key");
            Check(db.CustomIcons.Count == 1, "custom icon");
            Check(db.KdfParameters.KdfUuid.Equals(new Argon2Kdf(Argon2Type.ID).Uuid), "Argon2id KDF kept");
            Check(db.KdfParameters.GetUInt64(Argon2Kdf.ParamIterations, 0) == 6, "Argon2 iterations kept");
            Check(db.RecycleBinEnabled && db.RootGroup.FindGroup(db.RecycleBinUuid, true) != null, "recycle bin");
            Check(db.DeletedObjects.UCount == 1, "deleted objects");

            PwGroup banking = null;
            foreach (PwGroup g in db.RootGroup.Groups) if (g.Name == "Banking & Finance") banking = g;
            Check(banking != null && banking.CustomData.Get("group-key") == "group-value", "group + group custom data");
            Check(banking != null && banking.EnableSearching == false && banking.EnableAutoType == null, "group search/auto-type flags");
            Check(db.RootGroup.EnableSearching == null && db.RootGroup.EnableAutoType == null, "root group inherits flags");
            PwGroup bin = db.RootGroup.FindGroup(db.RecycleBinUuid, true);
            Check(bin != null && bin.EnableAutoType == false && bin.Entries.UCount == 1, "recycle bin flags + entry");
            Check(banking != null && banking.DefaultAutoTypeSequence == "{USERNAME}{TAB}{PASSWORD}", "group auto-type sequence");
            Check(banking != null && banking.Groups.UCount == 1 && !banking.Groups.GetAt(0).CustomIconUuid.Equals(PwUuid.Zero), "group custom icon");
            Check(banking != null && banking.Groups.UCount == 1 && banking.Groups.GetAt(0).Entries.UCount == 1, "nested group + entry");

            PwEntry e = banking == null ? null : banking.Entries.GetAt(0);
            Check(e != null && e.Strings.ReadSafe(PwDefs.TitleField) == "Bank <main> & \"co\"", "title with XML characters");
            Check(e != null && e.Strings.ReadSafe(PwDefs.PasswordField) == "rust-password", "password edited in Rust");
            Check(e != null && e.Strings.Get(PwDefs.PasswordField).IsProtected, "password still protected");
            Check(e != null && e.Strings.ReadSafe(PwDefs.NotesField) == "line 1\r\nline 2 é中\U0001F511" ||
                  e != null && e.Strings.ReadSafe(PwDefs.NotesField) == "line 1\nline 2 é中\U0001F511", "notes with unicode");
            Check(e != null && e.Strings.ReadSafe("PIN") == "1234" && e.Strings.Get("PIN").IsProtected, "protected custom field");
            Check(e != null && e.Strings.Get(PwDefs.NotesField).IsProtected, "notes protected by the Rust app read as protected");
            Check(e != null && e.Strings.ReadSafe("Client secret") == "from-rust" && e.Strings.Get("Client secret").IsProtected, "protected field added in Rust");
            Check(e != null && e.Strings.Get(PwDefs.TitleField) != null && !e.Strings.Get(PwDefs.TitleField).IsProtected, "title stays unprotected");
            Check(e != null && e.Strings.ReadSafe("Account") == "000-111", "custom field");
            Check(e != null && e.History.UCount == 3, "three history versions after the Rust edit");
            Check(e != null && e.History.UCount == 3
                  && e.History.GetAt(0).Strings.ReadSafe(PwDefs.PasswordField) == "old-password"
                  && e.History.GetAt(1).Strings.ReadSafe(PwDefs.PasswordField) == "middle-password"
                  && e.History.GetAt(2).Strings.ReadSafe(PwDefs.PasswordField) == "current-password", "history oldest first, Rust edit appended last");
            Check(e != null && e.History.UCount == 3 && e.History.GetAt(2).Binaries.Get("statement.txt") != null, "attachment kept in new history version");
            Check(e != null && e.Binaries.Get("statement.txt") != null &&
                  Encoding.UTF8.GetString(e.Binaries.Get("statement.txt").ReadData()) == "hello attachment", "attachment");
            Check(e != null && e.Binaries.Get("secret.bin") != null && e.Binaries.Get("secret.bin").ReadData().Length == 300, "binary attachment");
            Check(e != null && e.AutoType.AssociationsCount == 1 && e.AutoType.DefaultSequence == "{USERNAME}{TAB}{PASSWORD}{ENTER}", "auto-type");
            Check(e != null && e.ForegroundColor.ToArgb() == Color.FromArgb(0xC0, 0x10, 0x20).ToArgb(), "entry colour");
            Check(e != null && e.OverrideUrl == "cmd://\"{URL}\"", "override URL");
            Check(e != null && e.Tags.Count == 2, "tags");
            Check(e != null && e.CustomData.Get("entry-key") == "entry-value", "entry custom data");
            Check(e != null && e.Expires && e.ExpiryTime == new DateTime(2030, 1, 2, 3, 4, 5, DateTimeKind.Utc), "expiry");
            Check(e != null && !e.QualityCheck, "quality check flag");

            // KeePass keeps the record, unread, when it saves the database itself.
            string keptPath = Path.Combine(dir, "keepass-resaved.kdbx");
            db.SaveAs(IOConnectionInfo.FromPath(keptPath), false, null);
            db.Close();
            PwDatabase kept = new PwDatabase();
            kept.Open(IOConnectionInfo.FromPath(keptPath), Key(), null);
            Check(record != null && kept.CustomData.Get(CredentialRecordKey) == record, "KeePass keeps the credential record on save");
            kept.Close();

            Envelope rustEnv = Envelope.Parse(File.ReadAllText(Path.Combine(dir, "rust.axvault"), Encoding.UTF8));
            byte[] k = rustEnv.Open(PrfOutput);
            bool same = k.Length == 32;
            for (int i = 0; same && i < 32; i++) same = k[i] == EnvelopeKey[i];
            Check(same, "Rust-sealed passkey file opens in the KeePass plugin code");

            List<Envelope> ring = Envelope.ParseAll(File.ReadAllText(Path.Combine(dir, "rust-ring.axvault"), Encoding.UTF8));
            bool ringOk = ring.Count == 2;
            for (int r = 0; ringOk && r < 2; r++)
            {
                byte[] rk = ring[r].Open(r == 0 ? PrfOutput : Fill(0x55, 32));
                for (int i = 0; ringOk && i < 32; i++) ringOk = rk[i] == EnvelopeKey[i];
            }
            Check(ringOk, "Rust multi-key passkey file: both keys open in the KeePass plugin code");

            Console.WriteLine(failures == 0 ? "ALL PASSED" : failures + " FAILED");
            return failures == 0 ? 0 : 1;
        }
    }
}
