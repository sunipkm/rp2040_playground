# RP2040 Playground with `probe-rs` and `elf2uf2`
1. Install the `udev` rules:
```sh
sudo install rules/*.rules /etc/udev/rules.d
```
2. Reload the rules:
```sh
sudo udevadm control --reload-rules && sudo udevadm trigger
```

### Resources
- [Embassy IRQ for RP2040](https://www.reddit.com/r/rust/comments/1haqrtz/embassy_rs_interrupts_for_the_rp2040/)
- Update `embassy-rp` to commit `5a19b64fec396db1ababa6d3e7a71f4b3c6bab18` to use the over/underclocking feature of the RP series MCUs.