# Contributing

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Terms for contributions

kith is offered under the [GNU AGPL v3.0](LICENSE) and, separately, under commercial licenses. So that both stay possible, these terms apply to every contribution that is merged into kith. By opening a pull request you agree that, if it is merged:

1. You wrote the contribution, or otherwise have the right to submit it.
2. You grant CLoaKY233 a perpetual, worldwide, irrevocable, royalty-free license to use, copy, modify, sublicense and distribute your contribution as part of kith, under the GNU AGPL v3.0, under commercial licenses, or under any other license.
3. You keep the copyright in your contribution and may use it in any other way you like.

A pull request that is closed without being merged grants nothing; the code stays entirely yours. If you cannot agree to these terms, please open an issue describing the change instead.

## Working on kith

Build and test:

```sh
cargo build --release -p kith-cli
cargo nextest run --workspace --all-features
```

`docs/engineering.md` covers the conventions the code follows. Keep tests few and meaningful, and commit messages plain and descriptive.
