import Dispatch
import Foundation
import Virtualization

final class Guest: NSObject, VZVirtualMachineDelegate {
    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        exit(0)
    }

    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        FileHandle.standardError.write(Data("Virtual machine stopped: \(error)\n".utf8))
        exit(1)
    }
}

guard CommandLine.arguments.count == 4 else {
    FileHandle.standardError.write(Data("Usage: boot-linux KERNEL INITRAMFS GUEST_GIB\n".utf8))
    exit(2)
}
guard VZVirtualMachine.isSupported else {
    FileHandle.standardError.write(Data("Virtualization.framework is unavailable.\n".utf8))
    exit(2)
}

let bootLoader = VZLinuxBootLoader(kernelURL: URL(fileURLWithPath: CommandLine.arguments[1]))
bootLoader.initialRamdiskURL = URL(fileURLWithPath: CommandLine.arguments[2])
bootLoader.commandLine = "console=hvc0 rdinit=/init quiet panic=1"

let serial = VZVirtioConsoleDeviceSerialPortConfiguration()
serial.attachment = VZFileHandleSerialPortAttachment(fileHandleForReading: nil,
                                                    fileHandleForWriting: .standardOutput)

let config = VZVirtualMachineConfiguration()
config.bootLoader = bootLoader
config.cpuCount = 2
guard let memory = UInt64(CommandLine.arguments[3]), [2, 4].contains(memory) else {
    FileHandle.standardError.write(Data("Guest memory must be 2 or 4 GiB.\n".utf8))
    exit(2)
}
config.memorySize = memory * 1024 * 1024 * 1024
config.serialPorts = [serial]
config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]
try config.validate()

let guest = Guest()
let machine = VZVirtualMachine(configuration: config)
machine.delegate = guest
machine.start { result in
    if case let .failure(error) = result {
        FileHandle.standardError.write(Data("Could not start Linux: \(error)\n".utf8))
        exit(1)
    }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 145) {
    FileHandle.standardError.write(Data("Linux proof timed out.\n".utf8))
    if machine.canStop {
        machine.stop { _ in exit(124) }
    } else {
        exit(124)
    }
}
RunLoop.main.run()
