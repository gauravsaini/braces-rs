# braces-rs

Fast Rust-powered drop-in replacement for [micromatch/braces](https://github.com/micromatch/braces) — brace expansion, 1.2–3.6x faster than the JS reference, with byte-identical output.

**API-identical to micromatch/braces**: `expand`, `compile`, `create`, `stringify`, `parse`, plus the callable default function and an extra batch API (`expandMany`).

```sh
npm install braces-rs
```

```js
const braces = require('braces-rs');

braces.expand('a/{b,c}'); // ['a/b', 'a/c']
braces.compile('{a,b,c}'); // '(a|b|c)'
braces.create('{1..3}', { expand: true }); // ['1', '2', '3']
braces('{a,b}/file-{1..3}.txt'); // ['(a|b)/file-(1|2|3).txt']
```

Full documentation, benchmarks and correctness evidence:
https://github.com/gauravsaini/braces-rs

## License

MIT © Gaurav Saini
