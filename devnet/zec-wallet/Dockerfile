# zcash-devtool with regtest support: the devnet's funded Zcash wallet.
FROM rust:1.93-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends libsqlite3-dev && rm -rf /var/lib/apt/lists/*
RUN cargo install --locked --features regtest_support \
    --git https://github.com/zcash/zcash-devtool --rev 0b3436bb8d235c67cd36b800cb45a7b1b489c415 zcash-devtool

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/cargo/bin/zcash-devtool /usr/local/bin/
COPY activation-heights.toml /activation-heights.toml
RUN mkdir /wallet
