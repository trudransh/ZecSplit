# Credits

ZecSplit builds on other people's work.

## Protocol

- Joël Gugger, *Bitcoin–Monero Cross-chain Atomic Swap* (2020), and its implementations by COMIT ([xmr-btc-swap](https://github.com/comit-network/xmr-btc-swap)) and [eigenwallet](https://github.com/eigenwallet/core). ZecSplit ports that protocol to shielded Zcash.

## Code in this repository

- [`vendor/dleq`](vendor/dleq): Serai's `dleq` 0.4.1 by Luke Parker and Lee Bousfield, unmodified, MIT. ZecSplit uses its experimental cross-group DLEq proof.
- [`vendor/pasta_curves`](vendor/pasta_curves): `pasta_curves` 0.5.1 by the Electric Coin Company, MIT or Apache-2.0, with a zeroize-only backport.
- `apps/web`: the hero and footer animations follow [stackedlol/ascii-sharks](https://github.com/stackedlol/ascii-sharks), MIT, notice below. The koi pond is inspired by [Dao-Ho/portfolio](https://github.com/Dao-Ho/portfolio) (daoho.dev); no code is copied from it.
- Coin logos from [Simple Icons](https://simpleicons.org), CC0.

### stackedlol/ascii-sharks

```
MIT License

Copyright (c) 2026 drewdi

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
```
