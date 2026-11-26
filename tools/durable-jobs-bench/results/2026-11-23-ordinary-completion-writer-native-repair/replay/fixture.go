package main

import (
	"bufio"
	"fmt"
	"os"
	"os/exec"
	"syscall"
	"time"
)

func guestInit() {
	defer syscall.Reboot(syscall.LINUX_REBOOT_CMD_POWER_OFF)
	if err := syscall.Mount("proc", "/proc", "proc", 0, ""); err != nil {
		panic(err)
	}
	if err := os.Chmod("/", 0755); err != nil {
		panic(err)
	}

	if err := os.Chmod("/tmp", 0777|os.ModeSticky); err != nil {
		panic(err)
	}
	cmd := exec.Command("/usr/bin/python3.14", "-B", "/writer-controls/linux-control.py")
	cmd.Stdout, cmd.Stderr = os.Stdout, os.Stderr
	cmd.Env = append(os.Environ(), "COMBINED_BULK_SOURCE=/writer-controls/bulk", "PYTHON_COLORS=0")
	cmd.SysProcAttr = &syscall.SysProcAttr{Credential: &syscall.Credential{Uid: 65534, Gid: 65534}}
	if err := cmd.Run(); err != nil {
		fmt.Printf("WRITER_LINUX_EXIT %v\n", err)
		return
	}
	fmt.Println("WRITER_LINUX_EXIT 0")
}

func main() {
	if os.Getpid() == 1 {
		guestInit()
		return
	}
	if os.Getenv("DUMPABLE") != "1" {
		// Linux PR_SET_DUMPABLE, with the same disabled setting as release Mesh.
		_, _, errno := syscall.Syscall6(syscall.SYS_PRCTL, 4, 0, 0, 0, 0, 0)
		if errno != 0 {
			panic(errno)
		}
	}
	if os.Args[1] == "session" {
		fmt.Println("SESSION_READY")
		input := bufio.NewScanner(os.Stdin)
		for input.Scan() {
			fmt.Println("{}")
			if input.Text() == "stop" {
				return
			}
		}
		return
	}
	fmt.Println("READY")
	for {
		time.Sleep(time.Second)
	}
}
