# RP2040 Playground with `probe-rs` and `elf2uf2`
1. Install the `udev` rules:
```sh
sudo install rules/*.rules /etc/udev/rules.d
```
2. Reload the rules:
```sh
sudo udevadm control --reload-rules && sudo udevadm trigger
```