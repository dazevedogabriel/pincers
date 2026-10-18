# How IDM segments downloads

Static analysis of `IDMan.exe` (x86 PE, SHA-256 `3f10e64f…37b9ee`) with REA + Ghidra 12.1.4 (headless).
The binary has no symbols. The internal names (`ChunksList::FindMaxFreeChunk`, etc.)
come from the error strings `"Unknown error during X::Y()"`. Each one is referenced by
the MSVC *catch funclet* placed right after the function that owns it, which lets us
map each name to its address.

> Legend: **[O]** observed directly in the decompiled code · **[I]** inference.

## Internal classes and addresses

| IDM name | Address | Role |
|---|---|---|
| `ChunksList::FindMaxFreeChunk` | 0x555290 | picks the chunk with the most free space |
| `ChunksList::NeedNewConnection` | 0x5565f0 | decides whether to open another connection |
| *"Insert next chunk in list"* | 0x55bb60 | splits a chunk and creates the new segment |
| `ChunksList::AddAlternative` | 0x55fda0 | "racing" connection on the same range |
| `ChunksList::JoinFile` / `AppendFile` | 0x55c610 / 0x559250 | joins the part files at the end |
| `HTTPConnection::GetTempEndPos` | 0x57b920 | dynamic end of each connection |
| request builder (`Range:`) | 0x585fb0 | builds the HTTP GET |
| `Session::CalcSpeed` | 0x5b4d80 | speed measurement |

## The algorithm

1. **First connection** sends `Range: bytes=0-` **[O]** (0x585fb0). A 206 with `Content-Range`
   gives the size and proves the server can resume.

2. **Free space of a chunk** (FindMaxFreeChunk) **[O]**:
   `free = end − (start + downloaded) + 1 − speed × t`
   where `speed` is the speed of the connection serving that chunk and `t` is a
   parameter passed in by the caller. **[I]** `t` is the time a new connection
   takes to establish, i.e. IDM discounts what the current connection will still
   download before the new one arrives. The chunk with the largest `free` wins.

3. **Split criterion** (NeedNewConnection) **[O]**: only if `free ≥ 2·MinSeg + 3`.
   The constants in 0x55bb60 give `MinSeg = 0x2DA0 = 11680 bytes`
   (= 8 × 1460, eight TCP segments) and `0x5B43 = 23363 = 2·11680 + 3`.

4. **Where to cut** (0x55bb60) **[O]**: `new_start = end − free/2`. The new segment takes
   the second half of the free space, and the old chunk's end becomes `new_start − 1`.
   Special cases **[O]**:
   * on the first split of a single-chunk file, the second connection goes to the **tail**
     of the file (last 11680 or 23360 bytes);
   * on ZIP files, the first split targets the central directory (`FindChunkWithZipCentralDir`)
     so the archive's contents can be listed early;
   * one variant limits the cut to `start + 10 × (downloaded + 11680)`, so the first connection
     doesn't hand off too much before its speed is known.

5. **Dynamic segmentation** **[O]**: NeedNewConnection is called from 8 places, including
   the end of each connection. Whenever a connection finishes, it immediately splits the
   largest remaining chunk, so all connections stay busy until the end.

6. **Dynamic end** (GetTempEndPos) **[O]**: a connection does not stop at its nominal end.
   It walks the list and treats as its limit the start of the next chunk **that has
   already received ≥ 0x33 (51) bytes**. Chunks whose connection hasn't produced data yet
   are absorbed.

7. **Overlap verification** **[O]** (0x585fb0): when a range is closed, the end is
   `GetTempEndPos + 0x40`, i.e. 64 bytes **past** the boundary. In some modes the request
   also starts *before* the position. **[I]** Those bytes are compared with what the neighbour
   downloaded. That matches the string *"IDM started to download the file using one connection
   because of data corruption when newly received data in several connections did not match"*.

8. **Storage** **[O]**: each chunk goes to its own file (`%s%ld` in the temp folder). At the end
   `JoinFile` concatenates the parts *("appending all downloaded parts into one file...")*.
   The resume state goes in the registry as `{ID, startPos, nextID}` records.

9. **Racing connections** **[O]** (`AddAlternative`/`ApplyAlternative`): a second connection
   can download the same range as a slow one; the one that finishes first wins.

## What Pincers reproduces

| Behaviour | Pincers |
|---|---|
| probe `bytes=0-` reused as the 1st connection | ✅ `probe.rs` |
| `free = remaining − speed × t` (t = measured connect time, EMA) | ✅ `chunks.rs::find_max_free_chunk` |
| split if `free ≥ 2·11680+3`, cut at `end − free/2` | ✅ `need_new_connection` / `split` |
| re-split when a connection finishes | ✅ scheduler in `engine.rs` |
| dynamic end + absorbing neighbours with < 51 bytes | ✅ `try_absorb_next` |
| 64-byte verification and fallback to 1 connection | ✅ `VERIFY_OVERLAP` |
| `If-Range` on resume, retries with back-off, timeout | ✅ |
| 8 connections by default (`MaxConnectionsNumber`) | ✅ `-c` |
| speed limiter | ✅ `limiter.rs` |

Deliberate differences:
* **One preallocated file with positional writes** instead of part files + `JoinFile`.
  It removes the "assembling" phase, which IDM itself warns can take a while on large files.
* Resume state in `<file>.pincers` (JSON) instead of the registry.
* Not implemented (yet): racing connections, the ZIP/tail special cases, FTP, browser integration.
