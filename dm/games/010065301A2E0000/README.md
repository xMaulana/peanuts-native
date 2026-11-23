# STAR OCEAN THE SECOND STORY R (`010065301A2E0000`)

<!-- PEANUTS: a Peanuts Native profile and note (Banana Native has none for this title). -->

Update 1.1.0 (v196608) over the base game, with 5 small DLC packages (`010065301A2E1001` to `…1005`, about 100 KB each), SDK 15.3.2. **Unity 2021.3.22f1** (IL2CPP) with CRIWARE (ADX/USM) and FMOD, rendering with **NVN**. Modules `main`, `subsdk0` (NintendoSDK_movie) and `sdk`. The closest profiles are [Blasphemous 2](../010040801A4C8000/README.md) (Unity 2021.3.20f1, also SDK 15 era with a 16 MB system resource) and [Sea of Stars](../01008C0016544000/README.md) (Unity 2021.3.25f1); [My Time at Sandrock](../0100B63016916000/README.md) (2021.3.26f1) for the pool size.

**Status: plays on an AYN Thor (Turnip v26.2.0-R5, driver ID 18), 2026-11-22.** Title (Ver. 1.10 notice), New Game, the Milokeenia opening events with two scene transitions, free movement with the minimap, the autosave loaded after a relaunch; 30 fps as on the Switch (1 % low 27, GPU about 7 ms), audio through AAudio with no underruns after start-up, no crash in about 25 minutes of runs over eleven starts. All 5 DLC are mounted (`DM_AOC_DIR`, 5 links); their items were not checked in game. The system driver was not tried (owner: Turnip only for this test). No companion exists for this title.

## Builds

| Module | Build ID |
|---|---|
| `main` | `d214ba64d902aeda4197b57debabd01b` |
| `subsdk0` | `368f01d5cb42f06cd125dc739d036719` |
| `sdk` | `8618895246b420329b77746b960a66ab` |

The update carries the whole game (exefs and the full RomFS, 3,027 files, 11.08 GB), so the `.nup` holds the update and the DLC, not the base package.

## Shaders

`shader_split.py` reads the `.nup` in place and finds 23,476 shaders (207 compute, 1 geometry, 11,684 pixel, 11,584 vertex), 11,683 variation pairs and no `[RED]` line: 23,465 from Unity's Shader and ComputeShader objects (`Data/data.unity3d`, which holds `globalgamemanagers.assets`, and the 2,856 UnityFS bundles in `Data/StreamingAssets`), 11 from the player's GLSLC outputs in `main`. The 2021.3 layouts `unityfs.py` already had read every object; no new type tree was needed for 2021.3.22f1. The split takes about 19 s.

## Memory and remaining assumptions

| Setting | Where | Why it is a guess |
|---|---|---|
| `mem_reserve 0x200000000`, `mem_alias 0x1d8000000`, `mem_stack 0x10000000` | [`game.tsv`](game.tsv) | My Time at Sandrock's values. The survey's guess (`mem_reserve 0x140000000`, `mem_alias 0x118000000`, what Blasphemous 2 and Sea of Stars use) ran out of alias *address space* about 35 s into boot on the Thor: `alias region mapped up to +4.26 GB (alias region 4.38 GB)`, then either a black screen with every thread asleep or `CRASH sig=11 addr=0x3fffe0 pc=main+0x2d9830` right after a 4 MB `MapPhysicalMemory` (Unity's allocator got an empty region and wrote at NULL + size − 0x20; [Size-aligned regions exhaust the alias address space](../../../docs/reference/memory.md#size-aligned-regions-exhaust-the-alias-address-space)). With the 7.375 GiB alias (`DM_ALIAS_SIZE=0x1d8000000` on the same APK) the peak was +4.75 GB and the game played on |
| `app_pool 0xCD500000` (about 3.21 GiB) | `game.tsv` | the standard application pool; Sandrock faulted with a 1.5 GiB pool |
| `sys_resource 0x1000000` | `game.tsv` | read from the NPDM (16 MB), not a guess |
| `romfs_marker Data/data.unity3d` | `game.tsv` | this build has no loose `Data/globalgamemanagers` (it is inside `data.unity3d`) |

## First start

The first start prepares the shader parts of 11,683 programs (27.9 s on the Thor, 0 missing); the second start prepares them again (6.5 s, the gameplay save drops the profile's parts, as on every game), the third skips it. The prewarm seed (`vk_prewarm.bin`, 69 pipeline keys from title, menus, the opening events and Milokeenia, Turnip, layout `3710792d`) is made in 1.5 s on a cold start with 0 ms of waiting on the frame path; later scenes are not in it yet.

## Frame rate

The Switch release targets 30 fps; Game owned (the default) keeps it. FPSLocker Warehouse lists this exact `main` (`D214BA64D902AEDA`, 1.1.0) as needing no patch, with no issues listed: <https://github.com/masagrator/FPSLocker-Warehouse>. The game steps by elapsed time (Unity's frame time): from the Milokeenia autosave a 3 s walk ends at the same place at the game's own 30 fps and with `DM_PRESENT_INTERVAL=display` at `DM_FPS_LIMIT=20` and `60` (AYN Thor, Turnip v26.2.0-R5, 2026-11-22; that scene ran at 59 fps, 1 % low 55, GPU about 7 ms at the 60 cap). So the profile offers the display-paced caps 20, 30 and 60 (`fps_limit_present 1`, `fps_modes 20,30,60`), the method of [Yoshi's Crafted World](../01006000040C2000/README.md). Not measured: 40, 45, 90 and 120 (the Thor's panel stays at 60 Hz), cutscene and battle timing at 60.
