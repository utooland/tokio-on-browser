### What is this ?

A example to run [tokio](https://github.com/tokio-rs/tokio) in browser. Used a forked [utooland/tokio](https://github.com/utooland/tokio) which spawns web worker for multi threads runtime, and a file system [tokio-fs-ext](https://github.com/utooland/tokio-fs-ext) based on [OPFS](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system). Steps to run:

```bash
## install nodejs
npm install
## install rust
npm run install-toolchain
npm run dev
npm run start
```

Now, tokio::task::spawn_blocking not worked.
