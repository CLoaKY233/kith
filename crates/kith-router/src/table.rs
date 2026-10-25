//! A longest-prefix-match table for IPv4 and IPv6.
//!
//! Each address family has its own path-compressed binary trie (a Patricia trie). Nodes live
//! in one `Vec` and refer to each other by `u32` index, so the structure is a flat arena:
//! there is no `Box` recursion, and lookup is a plain loop with no recursion and no
//! allocation.
//!
//! Why this structure: for tens to tens of thousands of routes, a path-compressed trie visits
//! at most one node per distinct prefix length on the path to the address (in practice a
//! handful), each visit being one mask, one compare and one bit test. A sorted `Vec` per
//! prefix length would need up to 33 or 129 binary searches; a multibit trie would be faster
//! still but uses far more memory per route and is harder to keep correct on removal.
//!
//! Complexity, with `W` the address width (32 or 128) and `n` the number of prefixes:
//!
//! | Operation | Time | Allocates |
//! |---|---|---|
//! | [`PrefixTable::lookup`] | `O(W)` worst case, `O(depth)` typical | never |
//! | [`PrefixTable::get`] | `O(W)` | never |
//! | [`PrefixTable::insert`] | `O(W)` | at most two arena nodes (amortized) |
//! | [`PrefixTable::remove`] | `O(W)` | never |
//! | [`PrefixTable::iter`] | `O(n)` total | a stack of at most `W + 1` indices |
//!
//! Memory is `O(n)`: path compression bounds the arena at `2n + 1` live nodes per family.
//! Freed nodes go on a free list and are reused by later inserts.

use alloc::vec::Vec;
use core::net::IpAddr;
use core::num::NonZeroU32;

use kith_types::{IpPrefix, Ipv4Prefix, Ipv6Prefix};

/// The bits of an address, most significant bit first.
trait Bits: Copy + Eq {
    /// Address width in bits.
    const WIDTH: u8;
    /// All-zero bits.
    const ZERO: Self;

    /// The leading `len` bits; the rest cleared. `len >= WIDTH` keeps everything.
    fn masked(self, len: u8) -> Self;
    /// Bit `index` counted from the most significant bit. Out of range reads as zero.
    fn bit(self, index: u8) -> usize;
    /// How many leading bits `self` and `other` share.
    fn common_len(self, other: Self) -> u8;
}

macro_rules! impl_bits {
    ($bits:ty, $width:literal) => {
        impl Bits for $bits {
            const WIDTH: u8 = $width;
            const ZERO: Self = 0;

            fn masked(self, len: u8) -> Self {
                let host_bits = Self::WIDTH.saturating_sub(len);
                // Shifting by the full width means a zero-length mask: nothing kept.
                self & <$bits>::MAX.checked_shl(u32::from(host_bits)).unwrap_or(0)
            }

            fn bit(self, index: u8) -> usize {
                let top = self.checked_shl(u32::from(index)).unwrap_or(0) >> ($width - 1);
                usize::from(top == 1)
            }

            fn common_len(self, other: Self) -> u8 {
                u8::try_from((self ^ other).leading_zeros()).unwrap_or(Self::WIDTH)
            }
        }
    };
}

impl_bits!(u32, 32);
impl_bits!(u128, 128);

/// One trie node. The root is always index 0, covers the zero-length prefix and is never
/// freed, which is why child links can be `NonZeroU32`.
///
/// Invariant: a non-root node either holds an entry or has two children. Nodes that are
/// neither are spliced out on removal, so the trie stays path compressed.
#[derive(Debug, Clone)]
struct Node<K, V> {
    /// The node's prefix bits, already masked to `len`.
    key: K,
    /// The node's prefix length.
    len: u8,
    /// Children by the bit at position `len`.
    children: [Option<NonZeroU32>; 2],
    /// The route stored at exactly this prefix, if any.
    entry: Option<(IpPrefix, V)>,
}

impl<K: Bits, V> Node<K, V> {
    const fn new(key: K, len: u8, entry: Option<(IpPrefix, V)>) -> Self {
        Self {
            key,
            len,
            children: [None, None],
            entry,
        }
    }

    fn child(&self, direction: usize) -> Option<NonZeroU32> {
        self.children.get(direction).copied().flatten()
    }

    fn set_child(&mut self, direction: usize, child: Option<NonZeroU32>) {
        if let Some(slot) = self.children.get_mut(direction) {
            *slot = child;
        }
    }
}

fn index(link: NonZeroU32) -> usize {
    // A `u32` always fits in `usize` on every target kith supports (32 and 64 bit).
    usize::try_from(link.get()).unwrap_or(usize::MAX)
}

/// The trie for one address family.
#[derive(Debug, Clone)]
struct Trie<K, V> {
    nodes: Vec<Node<K, V>>,
    free: Vec<NonZeroU32>,
}

impl<K: Bits, V> Trie<K, V> {
    fn new() -> Self {
        Self {
            nodes: alloc::vec![Node::new(K::ZERO, 0, None)],
            free: Vec::new(),
        }
    }

    fn node(&self, link: NonZeroU32) -> Option<&Node<K, V>> {
        self.nodes.get(index(link))
    }

    fn node_mut(&mut self, link: NonZeroU32) -> Option<&mut Node<K, V>> {
        self.nodes.get_mut(index(link))
    }

    fn root(&self) -> Option<&Node<K, V>> {
        self.nodes.first()
    }

    /// Mutable access to a parent, where `None` means the root.
    fn parent_mut(&mut self, parent: Option<NonZeroU32>) -> Option<&mut Node<K, V>> {
        match parent {
            Some(link) => self.node_mut(link),
            None => self.nodes.first_mut(),
        }
    }

    fn allocate(&mut self, node: Node<K, V>) -> NonZeroU32 {
        if let Some(link) = self.free.pop()
            && let Some(slot) = self.node_mut(link)
        {
            *slot = node;
            return link;
        }
        let position = self.nodes.len();
        self.nodes.push(node);
        #[expect(
            clippy::expect_used,
            reason = "two nodes per prefix: four billion nodes would need far more memory \
                      than any device has, and index 0 is always the root"
        )]
        u32::try_from(position)
            .ok()
            .and_then(NonZeroU32::new)
            .expect("the arena holds fewer than u32::MAX nodes and slot 0 is the root")
    }

    fn release(&mut self, link: NonZeroU32) -> Option<(IpPrefix, V)> {
        let node = self.node_mut(link)?;
        node.children = [None, None];
        let entry = node.entry.take();
        self.free.push(link);
        entry
    }

    fn lookup(&self, addr: K) -> Option<(&IpPrefix, &V)> {
        let mut node = self.root()?;
        let mut best = node.entry.as_ref();
        while node.len < K::WIDTH {
            let Some(child) = node
                .child(addr.bit(node.len))
                .and_then(|link| self.node(link))
            else {
                break;
            };
            if addr.masked(child.len) != child.key {
                break;
            }
            if child.entry.is_some() {
                best = child.entry.as_ref();
            }
            node = child;
        }
        best.map(|(prefix, value)| (prefix, value))
    }

    /// Finds the node holding exactly `key/len`, with its parent link (`None` = root) and
    /// grandparent link, for removal.
    fn find(&self, key: K, len: u8) -> Option<Path> {
        let mut path = Path::default();
        let mut node = self.root()?;
        while node.len < len {
            let direction = key.bit(node.len);
            let link = node.child(direction)?;
            let child = self.node(link)?;
            if child.len > len || key.masked(child.len) != child.key {
                return None;
            }
            path.grandparent = path.parent;
            path.parent = path.node;
            path.node = Some(link);
            node = child;
        }
        (node.len == len).then_some(path)
    }

    fn get(&self, key: K, len: u8) -> Option<&V> {
        let path = self.find(key, len)?;
        let node = match path.node {
            Some(link) => self.node(link)?,
            None => self.root()?,
        };
        node.entry.as_ref().map(|(_, value)| value)
    }

    fn insert(&mut self, key: K, len: u8, prefix: IpPrefix, value: V) -> Option<V> {
        let mut parent: Option<NonZeroU32> = None;
        loop {
            let current = self.parent_mut(parent)?;
            if current.len == len {
                return current.entry.replace((prefix, value)).map(|(_, old)| old);
            }
            let direction = key.bit(current.len);
            let Some(link) = current.child(direction) else {
                let leaf = self.allocate(Node::new(key, len, Some((prefix, value))));
                self.parent_mut(parent)?.set_child(direction, Some(leaf));
                return None;
            };
            let child = self.node(link)?;
            let (child_key, child_len) = (child.key, child.len);
            let common = key.common_len(child_key).min(child_len).min(len);
            if common == child_len {
                parent = Some(link);
                continue;
            }
            // The new prefix and the child diverge (or the new prefix is shorter): put a
            // node at the shared length between the parent and the child.
            let split = if common == len {
                let mut node = Node::new(key, len, Some((prefix, value)));
                node.set_child(child_key.bit(len), Some(link));
                self.allocate(node)
            } else {
                let leaf = self.allocate(Node::new(key, len, Some((prefix, value))));
                let mut glue = Node::new(key.masked(common), common, None);
                glue.set_child(key.bit(common), Some(leaf));
                glue.set_child(child_key.bit(common), Some(link));
                self.allocate(glue)
            };
            self.parent_mut(parent)?.set_child(direction, Some(split));
            return None;
        }
    }

    fn remove(&mut self, key: K, len: u8) -> Option<V> {
        let path = self.find(key, len)?;
        let Some(link) = path.node else {
            return self.nodes.first_mut()?.entry.take().map(|(_, value)| value);
        };
        let node = self.node_mut(link)?;
        let (_, value) = node.entry.take()?;
        let [left, right] = node.children;
        let direction = key.bit(self.parent_mut(path.parent)?.len);
        match (left, right) {
            (Some(_), Some(_)) => {}
            (Some(only), None) | (None, Some(only)) => {
                self.parent_mut(path.parent)?
                    .set_child(direction, Some(only));
                self.release(link);
            }
            (None, None) => {
                self.parent_mut(path.parent)?.set_child(direction, None);
                self.release(link);
                // The parent may now be an entry-less node with one child: splice it out.
                if let Some(parent_link) = path.parent {
                    let parent = self.node(parent_link)?;
                    if parent.entry.is_none() {
                        let remaining = parent.child(0).or(parent.child(1));
                        let parent_direction = key.bit(self.parent_mut(path.grandparent)?.len);
                        self.parent_mut(path.grandparent)?
                            .set_child(parent_direction, remaining);
                        self.release(parent_link);
                    }
                }
            }
        }
        Some(value)
    }
}

/// Links to a node and its ancestors; `None` means the root.
#[derive(Debug, Default, Clone, Copy)]
struct Path {
    node: Option<NonZeroU32>,
    parent: Option<NonZeroU32>,
    grandparent: Option<NonZeroU32>,
}

/// A longest-prefix-match table mapping IPv4 and IPv6 prefixes to values.
///
/// Lookups never allocate. See the [module documentation](self) for the data structure and
/// complexity.
#[derive(Debug, Clone)]
pub struct PrefixTable<V> {
    v4: Trie<u32, V>,
    v6: Trie<u128, V>,
    len: usize,
}

impl<V> Default for PrefixTable<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> PrefixTable<V> {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            v4: Trie::new(),
            v6: Trie::new(),
            len: 0,
        }
    }

    /// Stores `value` at exactly `prefix`, returning the value it replaced.
    pub fn insert(&mut self, prefix: IpPrefix, value: V) -> Option<V> {
        let old = match prefix {
            IpPrefix::V4(p) => self.v4.insert(v4_key(p), p.prefix_len(), prefix, value),
            IpPrefix::V6(p) => self.v6.insert(v6_key(p), p.prefix_len(), prefix, value),
        };
        if old.is_none() {
            self.len = self.len.saturating_add(1);
        }
        old
    }

    /// Removes the value stored at exactly `prefix`.
    pub fn remove(&mut self, prefix: &IpPrefix) -> Option<V> {
        let old = match *prefix {
            IpPrefix::V4(p) => self.v4.remove(v4_key(p), p.prefix_len()),
            IpPrefix::V6(p) => self.v6.remove(v6_key(p), p.prefix_len()),
        };
        if old.is_some() {
            self.len = self.len.saturating_sub(1);
        }
        old
    }

    /// The value stored at exactly `prefix`.
    pub fn get(&self, prefix: &IpPrefix) -> Option<&V> {
        match *prefix {
            IpPrefix::V4(p) => self.v4.get(v4_key(p), p.prefix_len()),
            IpPrefix::V6(p) => self.v6.get(v6_key(p), p.prefix_len()),
        }
    }

    /// The longest stored prefix containing `addr`, with its value. Never allocates.
    pub fn lookup(&self, addr: IpAddr) -> Option<(&IpPrefix, &V)> {
        match addr {
            IpAddr::V4(addr) => self.v4.lookup(u32::from(addr)),
            IpAddr::V6(addr) => self.v6.lookup(u128::from(addr)),
        }
    }

    /// The number of stored prefixes.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the table holds no prefixes.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every entry in [`IpPrefix`] order: IPv4 before IPv6, then by address, then shorter
    /// prefixes first. The order depends only on the contents, never on insertion order.
    pub fn iter(&self) -> Iter<'_, V> {
        Iter {
            v4: TrieIter::new(&self.v4),
            v6: TrieIter::new(&self.v6),
        }
    }
}

impl<'a, V> IntoIterator for &'a PrefixTable<V> {
    type Item = (&'a IpPrefix, &'a V);
    type IntoIter = Iter<'a, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

fn v4_key(prefix: Ipv4Prefix) -> u32 {
    u32::from(prefix.addr())
}

fn v6_key(prefix: Ipv6Prefix) -> u128 {
    u128::from(prefix.addr())
}

/// Pre-order walk of one trie, `0` children before `1` children, which is prefix order.
#[derive(Debug)]
struct TrieIter<'a, K, V> {
    trie: &'a Trie<K, V>,
    stack: Vec<Option<NonZeroU32>>,
}

impl<'a, K: Bits, V> TrieIter<'a, K, V> {
    fn new(trie: &'a Trie<K, V>) -> Self {
        Self {
            trie,
            stack: alloc::vec![None],
        }
    }
}

impl<'a, K: Bits, V> Iterator for TrieIter<'a, K, V> {
    type Item = (&'a IpPrefix, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(link) = self.stack.pop() {
            let node = match link {
                Some(link) => self.trie.node(link),
                None => self.trie.root(),
            }?;
            let [left, right] = node.children;
            self.stack.extend(right.map(Some));
            self.stack.extend(left.map(Some));
            if let Some((prefix, value)) = &node.entry {
                return Some((prefix, value));
            }
        }
        None
    }
}

/// Iterator over a [`PrefixTable`], from [`PrefixTable::iter`].
#[derive(Debug)]
pub struct Iter<'a, V> {
    v4: TrieIter<'a, u32, V>,
    v6: TrieIter<'a, u128, V>,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (&'a IpPrefix, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        self.v4.next().or_else(|| self.v6.next())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::collections::BTreeMap;
    use alloc::vec::Vec;
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use proptest::prelude::*;

    use super::*;

    fn p(text: &str) -> IpPrefix {
        text.parse().unwrap()
    }

    fn a(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn hit(table: &PrefixTable<u32>, addr: IpAddr) -> Option<(IpPrefix, u32)> {
        table.lookup(addr).map(|(prefix, value)| (*prefix, *value))
    }

    #[test]
    fn nested_ipv4_prefixes_pick_the_longest() {
        let mut table = PrefixTable::new();
        table.insert(p("0.0.0.0/0"), 0);
        table.insert(p("10.0.0.0/8"), 8);
        table.insert(p("10.1.0.0/16"), 16);
        table.insert(p("10.1.2.3/32"), 32);
        assert_eq!(hit(&table, a("10.1.2.3")), Some((p("10.1.2.3/32"), 32)));
        assert_eq!(hit(&table, a("10.1.2.4")), Some((p("10.1.0.0/16"), 16)));
        assert_eq!(hit(&table, a("10.2.0.0")), Some((p("10.0.0.0/8"), 8)));
        assert_eq!(hit(&table, a("11.0.0.0")), Some((p("0.0.0.0/0"), 0)));
        assert_eq!(hit(&table, a("::1")), None, "families never mix");
        assert_eq!(table.len(), 4);
    }

    #[test]
    fn nested_ipv6_prefixes_pick_the_longest() {
        let mut table = PrefixTable::new();
        table.insert(p("::/0"), 0);
        table.insert(p("fd7a::/48"), 48);
        table.insert(p("fd7a::1/128"), 128);
        table.insert(p("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128"), 1);
        assert_eq!(hit(&table, a("fd7a::1")), Some((p("fd7a::1/128"), 128)));
        assert_eq!(hit(&table, a("fd7a::2")), Some((p("fd7a::/48"), 48)));
        assert_eq!(hit(&table, a("2001:db8::1")), Some((p("::/0"), 0)));
        assert_eq!(
            hit(&table, a("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")),
            Some((p("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/128"), 1))
        );
        assert_eq!(hit(&table, a("10.0.0.1")), None);
    }

    #[test]
    fn remove_restores_the_fallback_route() {
        let mut table = PrefixTable::new();
        table.insert(p("10.0.0.0/8"), 8);
        table.insert(p("10.1.0.0/16"), 16);
        table.insert(p("10.1.1.0/24"), 24);
        assert_eq!(table.remove(&p("10.1.0.0/16")), Some(16));
        assert_eq!(hit(&table, a("10.1.2.0")), Some((p("10.0.0.0/8"), 8)));
        assert_eq!(hit(&table, a("10.1.1.1")), Some((p("10.1.1.0/24"), 24)));
        assert_eq!(table.remove(&p("10.1.1.0/24")), Some(24));
        assert_eq!(hit(&table, a("10.1.1.1")), Some((p("10.0.0.0/8"), 8)));
        assert_eq!(table.remove(&p("10.1.1.0/24")), None);
        assert_eq!(table.len(), 1);
    }

    fn v4_prefix() -> impl Strategy<Value = IpPrefix> {
        // A narrow address space makes nesting and collisions common.
        (
            prop::sample::select(alloc::vec![
                0u32,
                0x0a00_0000,
                0x0a01_0000,
                0x0a01_0100,
                u32::MAX
            ]),
            any::<u32>(),
            0u8..=32,
        )
            .prop_map(|(base, noise, len)| {
                let addr = Ipv4Addr::from(base ^ (noise & 0x0000_ffff));
                IpPrefix::V4(Ipv4Prefix::truncating(addr, len).unwrap())
            })
    }

    fn v6_prefix() -> impl Strategy<Value = IpPrefix> {
        (
            prop::sample::select(alloc::vec![0u128, 0xfd7a << 112, u128::MAX]),
            any::<u16>(),
            0u8..=128,
        )
            .prop_map(|(base, noise, len)| {
                let addr = Ipv6Addr::from(base ^ (u128::from(noise) << 100) ^ u128::from(noise));
                IpPrefix::V6(Ipv6Prefix::truncating(addr, len).unwrap())
            })
    }

    fn prefix() -> impl Strategy<Value = IpPrefix> {
        prop_oneof![v4_prefix(), v6_prefix()]
    }

    fn addr() -> impl Strategy<Value = IpAddr> {
        // Addresses inside a random prefix, so lookups usually hit something.
        (prefix(), any::<u128>()).prop_map(|(prefix, noise)| match prefix {
            IpPrefix::V4(p) => {
                let noise = u32::try_from(noise & u128::from(u32::MAX)).unwrap();
                let host = noise & !u32::MAX.masked(p.prefix_len());
                IpAddr::V4(Ipv4Addr::from(u32::from(p.addr()) | host))
            }
            IpPrefix::V6(p) => {
                let host = noise & !u128::MAX.masked(p.prefix_len());
                IpAddr::V6(Ipv6Addr::from(u128::from(p.addr()) | host))
            }
        })
    }

    fn naive(routes: &BTreeMap<IpPrefix, u32>, addr: IpAddr) -> Option<(IpPrefix, u32)> {
        routes
            .iter()
            .filter(|(prefix, _)| prefix.contains(addr))
            .max_by_key(|(prefix, _)| prefix.prefix_len())
            .map(|(prefix, value)| (*prefix, *value))
    }

    #[derive(Debug, Clone)]
    enum Op {
        Insert(IpPrefix, u32),
        Remove(IpPrefix),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            3 => (prefix(), any::<u32>()).prop_map(|(prefix, value)| Op::Insert(prefix, value)),
            2 => prefix().prop_map(Op::Remove),
        ]
    }

    proptest! {
        #[test]
        fn lookup_matches_linear_scan(
            routes in prop::collection::btree_map(prefix(), any::<u32>(), 0..64),
            addrs in prop::collection::vec(addr(), 1..64),
        ) {
            let mut table = PrefixTable::new();
            for (prefix, value) in &routes {
                table.insert(*prefix, *value);
            }
            for addr in addrs {
                prop_assert_eq!(hit(&table, addr), naive(&routes, addr));
            }
        }

        #[test]
        fn operations_match_btreemap(
            ops in prop::collection::vec(op(), 0..128),
            addrs in prop::collection::vec(addr(), 1..32),
        ) {
            let mut table = PrefixTable::new();
            let mut model = BTreeMap::new();
            for op in ops {
                match op {
                    Op::Insert(prefix, value) => {
                        prop_assert_eq!(table.insert(prefix, value), model.insert(prefix, value));
                    }
                    Op::Remove(prefix) => {
                        prop_assert_eq!(table.remove(&prefix), model.remove(&prefix));
                    }
                }
                prop_assert_eq!(table.len(), model.len());
            }
            let entries: Vec<(IpPrefix, u32)> = table.iter().map(|(k, v)| (*k, *v)).collect();
            let expected: Vec<(IpPrefix, u32)> = model.iter().map(|(k, v)| (*k, *v)).collect();
            prop_assert_eq!(entries, expected);
            for prefix in model.keys() {
                prop_assert_eq!(table.get(prefix), model.get(prefix));
            }
            for addr in addrs {
                prop_assert_eq!(hit(&table, addr), naive(&model, addr));
            }
        }
    }
}
