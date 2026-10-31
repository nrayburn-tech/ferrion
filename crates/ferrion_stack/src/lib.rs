//! Stack growth for recursive code (docs/design/parser-deep-nesting.md).
//!
//! Every recursive function whose depth follows the input, in the parser and in every later phase,
//! calls [`ensure_stack`] on entry. When the current stack is nearly used up, it runs the function
//! on a new stack segment allocated on the heap, so deeply nested input never overflows the stack,
//! whatever thread the code runs on. The check is a comparison of the stack pointer, cheap enough
//! for every call.
//!
//! It wraps the `stacker` crate, which keeps its `unsafe` code and a little assembly per platform
//! (through `psm`) inside the dependency. This crate holds the dependency and its constants in one
//! place, so that crates which don't depend on the parser can use it too.

/// When less than this much stack is left, [`ensure_stack`] switches to a new segment. It must
/// cover the deepest stack any recursive function uses between two calls to `ensure_stack`.
pub const RED_ZONE: usize = 128 * 1024;

/// The size of each new stack segment.
pub const SEGMENT_SIZE: usize = 4 * 1024 * 1024;

/// Runs `f`, on a new stack segment if less than [`RED_ZONE`] bytes of stack are left.
#[inline]
pub fn ensure_stack<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(RED_ZONE, SEGMENT_SIZE, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recurses `depth` times, using a few hundred bytes of stack in each frame.
    fn recurse(depth: u32) -> u64 {
        ensure_stack(|| {
            let frame = [u64::from(depth); 32];
            if depth == 0 {
                0
            } else {
                std::hint::black_box(&frame);
                recurse(depth - 1) + frame[depth as usize % 32] % 2
            }
        })
    }

    #[test]
    fn deep_recursion_on_a_small_stack() {
        // A million frames of a few hundred bytes each need hundreds of MiB of stack, far more
        // than the thread's 256 KiB.
        let result = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| recurse(1_000_000))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(result, 500_000);
    }

    #[test]
    fn returns_the_value() {
        assert_eq!(ensure_stack(|| 42), 42);
    }
}
