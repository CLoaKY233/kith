# Contributing

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Terms for contributions

kith is offered under the [PolyForm Noncommercial License](LICENSE.md) and, separately, under commercial licenses. So that both stay possible, opening a pull request means you agree that:

1. You wrote the contribution, or otherwise have the right to submit it.
2. You grant CLoaKY233 a perpetual, worldwide, irrevocable, royalty-free license to use, copy, modify, sublicense and distribute your contribution as part of kith, under the PolyForm Noncommercial License, under commercial licenses, or under any other license.
3. You keep the copyright in your contribution and may use it in any other way you like.

If you cannot agree to this, please open an issue describing the change instead of a pull request.

## Working on kith

Build and test:

```sh
cargo build --release -p kith-cli
cargo nextest run --workspace --all-features
```

`docs/engineering.md` covers the conventions the code follows. Keep tests few and meaningful, and commit messages plain and descriptive.
