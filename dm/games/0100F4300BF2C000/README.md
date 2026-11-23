# New Pokemon Snap

<!-- PEANUTS: a Peanuts Native profile and note (Banana Native has none for this title). -->

**Status (current): runs on Android on Turnip (the contributor's report, below); the system driver is untested.**

- SDK 10, NVN + Wwise, Bandai Namco nu/nv.
- RomFS 3153 files, 5.81 GB; 724 new-only imports.
- Shaders: 3316 (8 compute, 2 geometry, 1743 pixel, 1563 vertex), all from the DRPF archives the RomFS keeps them in (shader_split.py's drp.py reader; only main's 8 GLSLC outputs sit outside them); 4689 variation pairs; dual-source blending for two keys.
- Memory: the game maps its own memory into the alias region and asks the SDK's region allocator for a 160-192 MB window at a park run and at the return to the lab, unchecked; the profile gives it 8 GB of alias in an 8.625 GB block (the first profile's 4.375 GB in 5 GB pinned the region table at +4.25 GB and the allocation failed, so the guest memset a null pointer). Two sessions past the old cap (the lab return and the park start) ran with no refusal.
- Build it on the phone. The PC builder (`peanuts/scripts/peanuts_build.py`) stops before the import with "build it on the phone; the PC builder can't decode its archives yet": the DRPF entries are Oodle-compressed, and no PC build from the game files has been checked against the profile's keys (it would need the ooz worker, `DM_OODLE_OOZ`, or the game's own decoder in a Mac host).

## Android on the AYN Thor

Joins the release in 0.1.7. The device results are the contributor's own report from pull request #308: the base game on an AYN Thor (Adreno 740, 12 GB, Android 13) with Turnip 27.0.0-R2 played the tutorial and a full park run, and the return to the lab and the next park start ran past the old memory cap with no crash and no missing shader. They have not been repeated on a maintainer's Thor. The system driver, frame rates and a first-launch prewarm seed are not checked; Game settings offers no frame rate rows for this game yet.
