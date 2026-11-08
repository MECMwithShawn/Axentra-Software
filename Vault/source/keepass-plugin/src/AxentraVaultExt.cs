// KeePass 2.x plugin: adds "Axentra Vault" as a key file source. The database key is a random
// 32-byte key file, wrapped in <database>.axvault under a passkey's PRF secret (e.g. an iPhone
// passkey over the Windows QR/Bluetooth link). A plain backup key file opens the database without it.
using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Windows.Forms;

using KeePass.Plugins;
using KeePassLib.Keys;

[assembly: AssemblyTitle("Axentra Vault")]
[assembly: AssemblyDescription("Unlock KeePass databases with a passkey (iPhone, Android, security key) via the WebAuthn PRF extension.")]
[assembly: AssemblyCompany("Axentra")]
[assembly: AssemblyProduct("KeePass Plugin")]
[assembly: AssemblyCopyright("Copyright © 2026 Axentra")]
[assembly: AssemblyVersion("0.1.0.0")]
[assembly: AssemblyFileVersion("0.1.0.0")]
[assembly: ComVisible(false)]

namespace AxentraVault
{
    public sealed class AxentraVaultExt : Plugin
    {
        private IPluginHost m_host;
        private PasskeyKeyProvider m_provider;

        public override bool Initialize(IPluginHost host)
        {
            if (host == null) return false;
            m_host = host;
            m_provider = new PasskeyKeyProvider();
            m_host.KeyProviderPool.Add(m_provider);
            return true;
        }

        public override void Terminate()
        {
            if (m_host != null && m_provider != null) m_host.KeyProviderPool.Remove(m_provider);
        }
    }

    public sealed class PasskeyKeyProvider : KeyProvider
    {
        internal const string ProviderName = "Axentra Vault";
        // Proven to work with iCloud Keychain passkeys over the Windows hybrid (QR) transport.
        internal const string RpId = "localhost";
        private const string Title = "Axentra Vault";

        [DllImport("user32.dll")]
        private static extern IntPtr GetForegroundWindow();

        public override string Name { get { return ProviderName; } }

        // The unwrapped bytes are used exactly like a KeePass 2.0 key file's data, which is
        // what lets the backup .keyx open the same database.
        public override bool DirectKey { get { return true; } }

        public override bool GetKeyMightShowGui { get { return true; } }

        // The Windows passkey dialog cannot appear on KeePass's secure desktop.
        public override bool SecureDesktopCompatible { get { return false; } }

        public override byte[] GetKey(KeyProviderQueryContext ctx)
        {
            try
            {
                string dbPath = ctx.DatabasePath;
                if (ctx.DatabaseIOInfo == null || !ctx.DatabaseIOInfo.IsLocalFile() || string.IsNullOrEmpty(dbPath))
                {
                    Error("Axentra Vault only supports databases stored as local files (OneDrive folders are fine).");
                    return null;
                }
                return ctx.CreatingNewKey ? Enroll(dbPath) : Unlock(dbPath);
            }
            catch (WebAuthnException ex)
            {
                if (!ex.IsCancellation) Error(ex.Message);
                return null;
            }
            catch (Exception ex)
            {
                Error(ex.Message);
                return null;
            }
        }

        private static byte[] Unlock(string dbPath)
        {
            string envPath = Envelope.PathFor(dbPath);
            if (!File.Exists(envPath))
            {
                Error("No passkey file found for this database:\r\n" + envPath +
                      "\r\n\r\nSelect your backup key file instead.");
                return null;
            }

            // Every registered key (phone passkey, YubiKey, ...) is offered; any one unlocks.
            List<Envelope> keys = Envelope.ParseAll(File.ReadAllText(envPath, Encoding.UTF8));
            string rpId = keys[0].RpId;
            keys = keys.FindAll(k => k.RpId == rpId);
            int used;
            byte[] prf = WebAuthn.EvaluatePrfAny(GetForegroundWindow(), rpId,
                keys.ConvertAll(k => k.CredentialId), keys.ConvertAll(k => k.PrfInput), out used);
            try
            {
                return keys[used].Open(prf);
            }
            catch (CryptographicException)
            {
                Error("That passkey does not unlock this database's passkey file:\r\n" + envPath +
                      "\r\n\r\nUse the passkey you registered for this database, or your backup key file.");
                return null;
            }
            finally { Envelope.Wipe(prf); }
        }

        private static byte[] Enroll(string dbPath)
        {
            string dbName = Path.GetFileName(dbPath);
            DialogResult go = MessageBox.Show(
                "Set up passkey unlock for " + dbName + ".\r\n\r\n" +
                "1. Windows asks where to save a new passkey. Choose \"iPhone, iPad, or Android device\" and scan the QR code.\r\n" +
                "2. Windows asks for the passkey again to confirm it works.\r\n" +
                "3. You save a backup key file. Keep it OFFLINE (USB stick or printout), not in OneDrive.\r\n\r\n" +
                "Without the passkey or the backup key file, this database cannot be opened.",
                Title, MessageBoxButtons.OKCancel, MessageBoxIcon.Information);
            if (go != DialogResult.OK) return null;

            IntPtr hWnd = GetForegroundWindow();
            WebAuthn.Registration reg = WebAuthn.Register(hWnd, RpId, "Axentra Vault for KeePass",
                WebAuthn.Random(16), "KeePass: " + dbName, "KeePass: " + dbName);
            // Windows' PRF flag is only a hint (security keys may not report it): the evaluation
            // below is the real test.
            byte[] prfInput = WebAuthn.Random(32);
            byte[] prf;
            try
            {
                prf = WebAuthn.EvaluatePrf(hWnd, RpId, reg.CredentialId, prfInput);
            }
            catch (WebAuthnException ex)
            {
                if (ex.ErrorName != "NotSupportedError") throw;
                Error("This passkey did not return a PRF secret" + (reg.PrfEnabled ? "" : " (PRF not reported at registration)") +
                      ", so it cannot unlock a database. Delete the passkey just created and use an iPhone, Android phone, " +
                      "or FIDO2 security key with hmac-secret (YubiKey 5 / Security Key series).");
                return null;
            }
            byte[] key = WebAuthn.Random(32);
            Envelope env;
            try
            {
                env = Envelope.Seal(RpId, reg.CredentialId, prfInput, prf, key);
                byte[] check = env.Open(prf);
                bool ok = check.Length == key.Length;
                for (int i = 0; ok && i < key.Length; i++) ok = check[i] == key[i];
                Envelope.Wipe(check);
                if (!ok) throw new CryptographicException("Key envelope self-check failed.");
            }
            finally { Envelope.Wipe(prf); }

            if (!SaveBackupKeyFile(dbPath, key))
            {
                Envelope.Wipe(key);
                return null;
            }

            WriteEnvelope(Envelope.PathFor(dbPath), env);
            MessageBox.Show(
                "Passkey unlock is ready. Finish by saving the database (Ctrl+S).\r\n\r\n" +
                "To open it: enter your master password, tick Key file and select \"" + ProviderName + "\".",
                Title, MessageBoxButtons.OK, MessageBoxIcon.Information);
            return key;
        }

        private static bool SaveBackupKeyFile(string dbPath, byte[] key)
        {
            while (true)
            {
                using (SaveFileDialog dlg = new SaveFileDialog())
                {
                    dlg.Title = "Save backup key file (keep it offline, not in OneDrive)";
                    dlg.Filter = "KeePass key file (*.keyx)|*.keyx";
                    dlg.FileName = Path.GetFileNameWithoutExtension(dbPath) + "-backup" + KeyFileWriter.DefaultExtension;
                    dlg.InitialDirectory = Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments);
                    dlg.OverwritePrompt = true;

                    if (dlg.ShowDialog() == DialogResult.OK)
                    {
                        if (IsInsideSyncedFolder(dlg.FileName) && MessageBox.Show(
                                "That location syncs to the cloud alongside your database, which weakens the backup.\r\n\r\n" +
                                "Save it there anyway?", Title, MessageBoxButtons.YesNo, MessageBoxIcon.Warning) != DialogResult.Yes)
                            continue;
                        KeyFileWriter.Write(dlg.FileName, key);
                        return true;
                    }
                }

                if (MessageBox.Show("A backup key file is required. Without it, losing the passkey means losing the database.\r\n\r\n" +
                        "Cancel passkey setup?", Title, MessageBoxButtons.YesNo, MessageBoxIcon.Warning) == DialogResult.Yes)
                    return false;
            }
        }

        private static bool IsInsideSyncedFolder(string path)
        {
            string full = Path.GetFullPath(path);
            foreach (string var in new string[] { "OneDrive", "OneDriveConsumer", "OneDriveCommercial" })
            {
                string root = Environment.GetEnvironmentVariable(var);
                if (!string.IsNullOrEmpty(root) && full.StartsWith(root.TrimEnd('\\') + "\\", StringComparison.OrdinalIgnoreCase))
                    return true;
            }
            return false;
        }

        // Keeps the previous envelope as .old until the new key is saved, in case the
        // master-key change is abandoned and the database still uses the old key.
        private static void WriteEnvelope(string envPath, Envelope env)
        {
            string tmp = envPath + ".tmp";
            File.WriteAllText(tmp, env.Serialize(), new UTF8Encoding(false));
            if (File.Exists(envPath))
            {
                string old = envPath + ".old";
                if (File.Exists(old)) File.Delete(old);
                File.Move(envPath, old);
            }
            File.Move(tmp, envPath);
        }

        private static void Error(string message)
        {
            MessageBox.Show(message, Title, MessageBoxButtons.OK, MessageBoxIcon.Error);
        }
    }
}
