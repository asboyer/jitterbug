# Third-party notices

## Stats

Jitterbug's native macOS memory accounting in
`src-tauri/src/platform/memory.rs` is adapted from
[Stats, Modules/RAM/readers.swift](https://github.com/exelban/stats/blob/21f866bb3e8ba606c9c01769f558accbcf1fa27c/Modules/RAM/readers.swift).
The implementation uses active, inactive, speculative, purgeable, external,
wired, and compressed page counts. It also converts swap page counters using
the kernel's actual page size. The Rust port adds checked arithmetic and missing-data handling.

Reference revision: `21f866bb3e8ba606c9c01769f558accbcf1fa27c`.
Stats' compact panels and visibility-aware rendering also informed the UI;
Jitterbug's components and monochrome menu bar glyph are original implementations.

### MIT License

Copyright (c) 2019 Serhiy Mytrovtsiy

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
