extern crate rand;

use std::mem;
use std::ptr;
use std::str;


#[derive(Debug)]
pub enum RopeError {
	PositionOutOfBounds,
	InvalidCodepoint,
}

pub trait Rope {
    fn new() -> Self;

    fn insert(&mut self, pos: usize, contents: &str) -> Result<(), RopeError>;
    fn del(&mut self, pos: usize, len: usize) -> Result<(), RopeError>;

    fn slice(&self, pos: usize, len: usize) -> Result<String, RopeError>;

	fn to_string(&self) -> String;

	fn len(&self) -> usize; // in bytes
	fn char_len(&self) -> usize; // in unicode values
}


// Must be <= UINT16_MAX. Benchmarking says this is pretty close to optimal
// (tested on a mac using clang 4.0 and x86_64).
//const NODE_SIZE: usize = 136;

// The likelyhood (%) a node will have height (n+1) instead of n
const BIAS: u32 = 25;

// The rope will become less efficient after the string is 2 ^ ROPE_MAX_HEIGHT nodes.
const MAX_HEIGHT: usize = 13;
const MAX_HEIGHT_U8: u8 = MAX_HEIGHT as u8;

#[derive(Clone)]
#[derive(Copy)]
struct SkipEntry {
	// The number of *characters* between the start of the current node and the
	// start of the next node.
	num_chars: usize,
    node: *mut Node,
}

fn max_bytes_per_node() -> usize { MAX_HEIGHT * mem::size_of::<SkipEntry>() }

// The node structure is designed in a very fancy way which would be more at home in C or something
// like that. The basic idea is that the node structure is fixed size in memory, but the proportion
// of that space taken up by characters and by the height are different depentant on a node's
// height.
#[repr(C)]
struct Node {
    // Height of skips array.
    height: u8,
	// Number of bytes in contents in use
	num_bytes: u8,

    // This is essentially a hand-spun union type. Any characters not used by height skips will be
    // filled with characters. (The height is 2.)
    skips: [SkipEntry; MAX_HEIGHT],
}

fn random_height() -> u8 {
    use rand::Rng;

    let mut rng = rand::thread_rng();

    let mut h = 1;
    while h < MAX_HEIGHT_U8 && rng.gen::<bool>() { h+=1; }
    h
}

pub struct JumpRope {
    // The total number of characters in the rope
	num_chars: usize,

	// The total number of bytes which the characters in the rope take up
	num_bytes: usize,

    // This node won't have any actual data in it, and its height is set to the max height of the
    // rope.
    skips: Node,
}


impl SkipEntry {
    fn new() -> Self {
        SkipEntry { num_chars: 0, node: ptr::null_mut() }
    }
}

impl Node {
    fn skip_entries_mut(&mut self) -> &mut [SkipEntry] {
        &mut self.skips[..self.height as usize]
    }

    fn skip_entries(&self) -> &[SkipEntry] {
        &self.skips[..self.height as usize]
    }

    fn capacity(&self) -> usize {
        (MAX_HEIGHT - self.height as usize) * mem::size_of::<SkipEntry>()
    }

    fn content(&self) -> &[u8] {
        unsafe {
            let start = (&self.skips as *const SkipEntry).offset(self.height as isize) as *const u8;
            std::slice::from_raw_parts(start, self.capacity())
        }
    }

    fn content_mut(&mut self) -> &mut [u8] {
        unsafe {
            let start = (&mut self.skips[0] as *mut SkipEntry).offset(self.height as isize) as *mut u8;
            std::slice::from_raw_parts_mut(start, self.capacity())
        }
    }

    fn new_with_height(height: u8) -> Node {
        //println!("height {} {}", height, max_height());
        assert!(height >= 1 && height <= MAX_HEIGHT_U8);

        let mut node = Node {
            height: height,
            num_bytes: 0,
            skips: unsafe { mem::uninitialized() },
        };

        for mut skip in node.skip_entries_mut() {
            // The entries are uninitialized memory.
            unsafe { ptr::write(skip, SkipEntry::new()); }
        }

        /*
        for mut byte in node.content_mut() {
            *byte = 0;
        }*/

        node
    }

    fn new() -> Node {
        Self::new_with_height(random_height())
    }

    fn to_str(&self) -> &str {
        let slice = &self.content()[..self.num_bytes as usize];
        // The contents must be valid utf8 content.
        str::from_utf8(slice).unwrap()
    }

    fn next(&self) -> Option<&Node> {
        unsafe { self.skips[0].node.as_ref() }
    }
}

struct RopeIter {
    skips: [SkipEntry; MAX_HEIGHT],
}

impl RopeIter {
    fn update_offsets(&mut self, height: usize, by: isize) {
        for i in 0..height {
            unsafe {
                // `as usize` here is weird and gross. I guess thats what the C equivalent does.
                // Because of wrapping its still correct...
                (*self.skips[i].node).skips[i].num_chars += by as usize;
            }
        }
    }
}

impl JumpRope {
    pub fn new() -> Self {
        JumpRope {
            num_chars: 0,
            num_bytes: 0,
            skips: Node::new_with_height(1),
        }
    }

    fn head(&self) -> Option<&Node> {
        self.skips.next()
    }
    
    // Internal function for navigating to a particular character offset in the rope.  The function
    // returns the list of nodes which point past the position, as well as offsets of how far into
    // their character lists the specified characters are.
    fn iter_at_char(&self, char_pos: usize) -> RopeIter {
        unsafe {
            assert!(char_pos <= self.num_chars);

            let mut e: *const Node = &self.skips;
            let mut height = (self.skips.height - 1) as usize;
            
            let mut offset = char_pos; // How many more chars to skip

            let mut iter = RopeIter { skips: [SkipEntry::new(); MAX_HEIGHT] };

            loop {
                let ref en = *e;
                let skip = en.skips[height].num_chars;
                if offset > skip {
                    // Go right.
                    assert!(e == &self.skips || en.num_bytes > 0);
                    offset -= skip;
                    e = en.skips[height].node;
                    assert!(!e.is_null()); // Unexpectedly reached the end
                } else {
                    // Record this and go down.
                    iter.skips[height] = SkipEntry {
                        num_chars: offset,
                        node: e as *mut Node, // This is pretty gross
                    };

                    if height == 0 {
                        break;
                    } else {
                        height -= 1;
                    }
                }
            }

            assert!(offset <= max_bytes_per_node());
            return iter;
        }
    }

    // Internal fn to create a new node at the specified iterator filled with the specified
    // content.
    fn insert_node_at(&mut self, iter: &mut RopeIter, contents: &str, num_chars: usize) {
        

    }
}

impl Rope for JumpRope {
    fn new() -> Self {
        JumpRope::new()
    }

	fn insert(&mut self, pos: usize, contents: &str) -> Result<(), RopeError> {
        if contents.len() == 0 { return Result::Ok(()); }

		unimplemented!();

	}
    fn del(&mut self, pos: usize, len: usize) -> Result<(), RopeError> {
		unimplemented!();
	}

    fn slice(&self, pos: usize, len: usize) -> Result<String, RopeError> {
	   	unimplemented!();
   	}
	fn to_string(&self) -> String {
        let mut content = String::with_capacity(self.num_bytes);

        // TODO: Rewrite this using the node iterator.
        let mut node: Option<&Node> = self.head();

        while let Some(n) = node {
            content.push_str(n.to_str());
            node = n.next();
        }

        content
	}
	fn len(&self) -> usize { self.num_bytes }
	fn char_len(&self) -> usize { self.num_chars }
}

