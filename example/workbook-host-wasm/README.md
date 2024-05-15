# Workbook Host WASM example

Simple POC for doing postcard-rpc over WebUSB ([PR #17](https://github.com/jamesmunns/postcard-rpc/pull/17))

## How to build

First off, get the required tools. You'll need

* wasm-pack
* node (npm or pnpm)
* webpack

```shell
rustup target add wasm32-unknown-unknown
pnpm install
#wasm-pack build --target web
RUSTFLAGS=--cfg=web_sys_unstable_api pnpx webpack-cli && pnpx serve dist
```
