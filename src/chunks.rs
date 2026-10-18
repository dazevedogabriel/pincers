//! Segment bookkeeping — the Pincers counterpart of IDM's `ChunksList`.
//!
//! Behaviour reconstructed from IDMan.exe (see `docs/IDM_ANALYSIS.md`):
//!
//! * `find_max_free_chunk` mirrors `ChunksList::FindMaxFreeChunk` (0x555290):
//!   for every running segment, the *free* space is what is still left to
//!   download **minus what its connection will download while a new connection
//!   is being established** (`speed * connect_time`).
//! * `need_new_connection` mirrors `ChunksList::NeedNewConnection` (0x5565f0):
//!   split only if `free >= 2 * MIN_SEGMENT + 3`.
//! * `split` mirrors "Insert next chunk in list" (0x55bb60): the new segment
//!   takes the second half of the free space, `new_start = end - free / 2`.
//! * `effective_end` mirrors `HTTPConnection::GetTempEndPos` (0x57b920): a
//!   connection keeps reading past its nominal end, through segments whose
//!   connections have not yet produced `MIN_STARTED_BYTES` bytes.

/// 0x2DA0 = 11680 bytes = 8 TCP segments of 1460 bytes. IDM's minimal chunk.
pub const MIN_SEGMENT: u64 = 0x2DA0;
/// 0x33: a segment that received fewer bytes than this is considered "not
/// started" and may be swallowed by the connection before it.
pub const MIN_STARTED_BYTES: u64 = 0x33;
/// 0x40: bytes read beyond the boundary to verify both connections agree.
pub const VERIFY_OVERLAP: u64 = 0x40;

pub type SegId = u64;

#[derive(Debug, Clone)]
pub struct Segment {
    pub id: SegId,
    pub start: u64,
    /// Next byte to be written.
    pub pos: u64,
    /// Exclusive end. May shrink (split) or grow (absorbing a neighbour).
    pub end: u64,
    /// A connection is currently assigned.
    pub active: bool,
    /// Measured speed of the assigned connection (bytes/s).
    pub speed: f64,
    /// The last connection on this segment gave up after all its retries.
    pub failed: bool,
}

impl Segment {
    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.pos)
    }
    pub fn done(&self) -> bool {
        self.pos >= self.end
    }
}

#[derive(Debug, Default)]
pub struct ChunksList {
    /// Always sorted by `start`; ranges never overlap.
    pub segs: Vec<Segment>,
    /// Segments absorbed by a neighbour; their workers must stop.
    cancelled: Vec<SegId>,
    next_id: SegId,
}

impl ChunksList {
    pub fn new(size: u64) -> Self {
        let mut l = Self::default();
        l.push(0, 0, size);
        l
    }

    pub fn from_saved(saved: &[(u64, u64, u64)]) -> Self {
        let mut l = Self::default();
        for &(start, pos, end) in saved {
            l.push(start, pos, end);
        }
        l.segs.sort_by_key(|s| s.start);
        l
    }

    fn push(&mut self, start: u64, pos: u64, end: u64) -> SegId {
        let id = self.next_id;
        self.next_id += 1;
        self.segs.push(Segment { id, start, pos, end, active: false, speed: 0.0, failed: false });
        id
    }

    pub fn get(&self, id: SegId) -> Option<&Segment> {
        self.segs.iter().find(|s| s.id == id)
    }
    pub fn get_mut(&mut self, id: SegId) -> Option<&mut Segment> {
        self.segs.iter_mut().find(|s| s.id == id)
    }
    fn index(&self, id: SegId) -> Option<usize> {
        self.segs.iter().position(|s| s.id == id)
    }

    pub fn downloaded(&self) -> u64 {
        self.segs.iter().map(|s| s.pos - s.start).sum()
    }
    pub fn all_done(&self) -> bool {
        self.segs.iter().all(Segment::done)
    }
    pub fn active_count(&self) -> usize {
        self.segs.iter().filter(|s| s.active).count()
    }

    /// Segments with bytes left and no connection (after an error or resume).
    pub fn orphan(&self) -> Option<SegId> {
        self.segs.iter().find(|s| !s.active && !s.done()).map(|s| s.id)
    }

    /// `ChunksList::FindMaxFreeChunk`: returns the running segment with the
    /// most free space and that amount, accounting for what it will still
    /// download during `connect_time` seconds.
    pub fn find_max_free_chunk(&self, connect_time: f64) -> Option<(SegId, u64)> {
        self.segs
            .iter()
            .filter(|s| s.active && !s.done())
            .map(|s| {
                let in_flight = (s.speed * connect_time) as u64;
                (s.id, s.remaining().saturating_sub(in_flight))
            })
            .max_by_key(|&(_, free)| free)
    }

    /// `ChunksList::NeedNewConnection`.
    pub fn need_new_connection(&self, connect_time: f64, min_segment: u64) -> Option<(SegId, u64)> {
        self.find_max_free_chunk(connect_time)
            .filter(|&(_, free)| free >= 2 * min_segment + 3)
    }

    /// Splits `victim`, giving the new segment the upper half of its free
    /// space. Returns the id of the new (inactive) segment.
    pub fn split(&mut self, victim: SegId, free: u64) -> Option<SegId> {
        let idx = self.index(victim)?;
        let v = &self.segs[idx];
        let new_start = v.end - free / 2;
        if new_start <= v.pos || new_start >= v.end {
            return None;
        }
        let end = v.end;
        self.segs[idx].end = new_start;
        let id = self.next_id;
        self.next_id += 1;
        self.segs.insert(
            idx + 1,
            Segment { id, start: new_start, pos: new_start, end, active: false, speed: 0.0, failed: false },
        );
        Some(id)
    }

    /// `HTTPConnection::GetTempEndPos` + takeover. Called when the connection
    /// of `id` reached its nominal end: if the following segment has not
    /// really started (fewer than `MIN_STARTED_BYTES` received), it is
    /// absorbed — this connection keeps reading and the other one is
    /// cancelled. Returns `true` when the range was extended.
    pub fn try_absorb_next(&mut self, id: SegId) -> bool {
        let Some(idx) = self.index(id) else { return false };
        let Some(next) = self.segs.get(idx + 1) else { return false };
        if self.segs[idx].end != next.start || next.pos - next.start >= MIN_STARTED_BYTES {
            return false;
        }
        // Bytes the next connection already wrote are simply rewritten by us.
        let next = self.segs.remove(idx + 1);
        self.segs[idx].end = next.end;
        self.cancelled.push(next.id);
        true
    }

    /// Throws away all progress and starts over with one segment. Old ids are
    /// tombstoned so their workers stop instead of touching the new segment.
    pub fn reset(&mut self, size: u64) -> SegId {
        let ids: Vec<_> = self.segs.drain(..).map(|s| s.id).collect();
        self.cancelled.extend(ids);
        self.push(0, 0, size)
    }

    pub fn is_cancelled(&self, id: SegId) -> bool {
        self.cancelled.contains(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(l: &mut ChunksList, id: SegId, speed: f64) {
        let s = l.get_mut(id).unwrap();
        s.active = true;
        s.speed = speed;
    }

    #[test]
    fn splits_the_largest_free_half() {
        let mut l = ChunksList::new(1_000_000);
        running(&mut l, 0, 0.0);
        let (victim, free) = l.need_new_connection(0.5, MIN_SEGMENT).unwrap();
        let new = l.split(victim, free).unwrap();
        assert_eq!(l.get(0).unwrap().end, 500_000);
        assert_eq!(l.get(new).unwrap().start, 500_000);
        assert_eq!(l.get(new).unwrap().end, 1_000_000);
    }

    #[test]
    fn subtracts_what_the_connection_downloads_while_connecting() {
        let mut l = ChunksList::new(1_000_000);
        running(&mut l, 0, 400_000.0); // 400 KB/s, 0.5 s to connect -> 200 KB in flight
        let (_, free) = l.find_max_free_chunk(0.5).unwrap();
        assert_eq!(free, 800_000);
        let new = l.split(0, free).unwrap();
        assert_eq!(l.get(new).unwrap().start, 600_000);
    }

    #[test]
    fn refuses_tiny_segments() {
        let mut l = ChunksList::new(2 * MIN_SEGMENT + 2);
        running(&mut l, 0, 0.0);
        assert!(l.need_new_connection(0.0, MIN_SEGMENT).is_none());
    }

    #[test]
    fn absorbs_neighbour_that_has_not_started() {
        let mut l = ChunksList::new(100_000);
        running(&mut l, 0, 0.0);
        let new = l.split(0, 100_000).unwrap();
        l.get_mut(new).unwrap().pos += MIN_STARTED_BYTES - 1;
        l.get_mut(0).unwrap().pos = 50_000;
        assert!(l.try_absorb_next(0));
        assert_eq!(l.get(0).unwrap().end, 100_000);
        assert!(l.is_cancelled(new));
    }

    #[test]
    fn keeps_neighbour_that_started() {
        let mut l = ChunksList::new(100_000);
        running(&mut l, 0, 0.0);
        let new = l.split(0, 100_000).unwrap();
        l.get_mut(new).unwrap().pos += MIN_STARTED_BYTES;
        l.get_mut(0).unwrap().pos = 50_000;
        assert!(!l.try_absorb_next(0));
    }
}
