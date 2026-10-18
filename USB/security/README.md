# Security and build status

Version 0.1.1 is a local preview build. The Windows executable and installer are unsigned. No VirusTotal result is claimed for these artifacts. Release hashes are recorded in [the version notes](../releases/0.1.1.md).

The 30 core tests operate on ordinary temporary files and synthetic device records. This update did not format, erase or write any physical USB. Actual device writing, read-back and firmware boot remain unvalidated on hardware; the app documentation describes those limits.

No user device-inventory reports, installation ISOs, local credentials or build outputs are included in the source folder. Review exported diagnostic reports before sharing because they can contain paths and device identifiers.
