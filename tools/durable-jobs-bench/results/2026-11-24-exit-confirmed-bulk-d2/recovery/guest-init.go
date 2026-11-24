package main

import (
	"fmt"
	"os"
	"os/exec"
	"syscall"
)

func main() {
	defer syscall.Reboot(syscall.LINUX_REBOOT_CMD_POWER_OFF)
	if err := syscall.Mount("proc", "/proc", "proc", 0, ""); err != nil {
		panic(err)
	}
	command := exec.Command("/usr/bin/python3.14", "-B", "/relay-regression/guest-init.py")
	command.Env = []string{"PATH=/usr/bin:/bin", "LC_ALL=C", "PYTHON_COLORS=0"}
	command.Stdout, command.Stderr = os.Stdout, os.Stderr
	if err := command.Run(); err != nil {
		fmt.Printf("RELAY_LAYOUT_FAILED %v\n", err)
	}
}
