// Writes a KeePass 2.x XML key file (format 2.0) holding exactly the 32-byte key the passkey
// unwraps. KeePass, KeePassXC, KeePassDX, KeePassium and Strongbox use these bytes directly,
// so this file opens the database in any of them without the plugin.
using System;
using System.IO;
using System.Security.Cryptography;
using System.Text;

namespace AxentraVault
{
    internal static class KeyFileWriter
    {
        public const string DefaultExtension = ".keyx";

        public static string Build(byte[] key)
        {
            if (key == null || key.Length != 32) throw new ArgumentException("Key must be 32 bytes.");

            string hex = BitConverter.ToString(key).Replace("-", "");
            string hash;
            using (SHA256 sha = SHA256.Create())
                hash = BitConverter.ToString(sha.ComputeHash(key), 0, 4).Replace("-", "");

            StringBuilder sb = new StringBuilder();
            sb.Append("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n");
            sb.Append("<KeyFile>\r\n");
            sb.Append("\t<Meta>\r\n\t\t<Version>2.0</Version>\r\n\t</Meta>\r\n");
            sb.Append("\t<Key>\r\n");
            sb.Append("\t\t<Data Hash=\"").Append(hash).Append("\">\r\n");
            for (int line = 0; line < 2; line++)
            {
                sb.Append("\t\t\t");
                for (int group = 0; group < 4; group++)
                {
                    if (group > 0) sb.Append(' ');
                    sb.Append(hex, (line * 4 + group) * 8, 8);
                }
                sb.Append("\r\n");
            }
            sb.Append("\t\t</Data>\r\n");
            sb.Append("\t</Key>\r\n");
            sb.Append("</KeyFile>\r\n");
            return sb.ToString();
        }

        public static void Write(string path, byte[] key)
        {
            File.WriteAllText(path, Build(key), new UTF8Encoding(false));
        }
    }
}
