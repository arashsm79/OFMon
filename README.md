# Offile-first Smart Energy Montoring
> Associated mobile app available [here](https://github.com/arashsm79/OFMonMobile).
* [Introduction](#Introduction)
* [Rust and ESP32](#Rust-and-ESP32)
* [Software Packages Needed for Rust Development on ESP32](#Software-Packages-Needed-for-Rust-Development-on-ESP32)
* [Filesystem](#Filesystem)
* [Measuring current and voltage using SCT sensors](#Measuring-current-and-voltage-using-SCT-sensors)
* [References](#References)



# Introduction
There have been numerous smart energy monitoring systems based on the Internet of things (IoT). In most of these systems, the end devices are directly connected to a network and are continuously sending out their data. This poses a real challenge in environments where a reliable wireless connection is not always available, e.g., inside containers, remote locations, or even in places where there is simply no proper infrastructure for wireless communication.

I have developed an offline-first energy monitoring system using ESP32 microcontrollers and their bleeding-edge Rust ecosystem to store data locally in their flash. The readings are collected from the devices by connecting to their access point and are saved on the local storage using a smartphone running a fork of the Thingsboard Flutter Mobile application. After collecting the telemetry data from the devices and when a network connection is available, the mobile app can flush the data of all the devices to the Thingsboards server, an IoT platform used for data collection and visualization. Rust is a type-safe and memory-safe system programming language, it provides its safety guarantees at compile time using rules that eliminate many memory-safety issues. There has been a significant adoption in using Rust for embedded development after companies like STMicrocontrollers and Espressif Systems introduced their community projects for enabling the use of the Rust programming language on their SoCs and modules.

A Current transformer is connected to the ADC subsystem of ESP32 microcontroller and is used to calculate the RMS current and voltage and kWh of the power line. These readings are periodically saved to flash storage. The device also creates a Wi-Fi access point using a unique SSID that is a combination of its MAC address and a predefined string. It then spins up a web server to which clients can request telemetry data along with other operations.

The filesystem used for flash storage is of utmost importance. We have chosen LittleFS since it is power- loss resilient and has wear leveling mechanisms for flash storage. It also uses constant RAM to work with any kind of data. Some parts of the flash storage have been reserved for Over-the-Air (OTA) updates, but the majority of the flash storage is formatted with the LittleFS file system. The devices are identified using their server-side representation token which is given to them on their first time initialization using the mobile app. The mobile app also maintains a list of the devices it has connected to and fetches their server-side information when a connection is available. This information is shown alongside the devices’ SSID when scanning for access points. After collecting the data from the devices and syncing it with the Thingsboard server, customers with a few devices or operators with tens of devices can view their data both on the mobile app and on the
Thingsboard web app.
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/213930124-dffb86d8-de19-46cb-a774-2703518b55a3.png" height="400"></p>
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/213930133-bd489857-3335-40f6-89e1-6c59d2b6c26b.png" height="400"></p>
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/213930161-31927390-57aa-4144-a046-2c2bfc2b6901.png" width="400"></p>
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/213930168-805bd5f9-acce-4bae-a7e6-09efd210719a.png" width="400"></p>

# Rust and ESP32
Rust programming language has attracted the attention of many companies today. This language allows programmers to write safe programs with concepts such as Ownership and Borrowing and new programming structures. During compilation, this language can guarantee that your program will not encounter many types of errors that were common in older languages such as C.
Espressif has designed a framework for programming its microcontrollers, which includes a set of libraries written in C and other tools written in Python. This framework is called [esp-idf](https://github.com/espressif/esp-idf), and there is a tutorial on how to install and start working with it on the company's website. This framework provides programmers with all the tools needed to set up a project in C language and easily use different parts of the microcontroller including Wifi and Bluetooth.

Over the last few years, the company has started a movement to make it possible to use esp-idf, which is written in C, in the Rust language using FFI, or Foreign Function Interface. Using FFI, code written in one language can be called in another language and its return value can be retrieved. All the work done in this regard is in the [esp-rs](https://github.com/esp-rs) repository and the development of all these programs is done as open source on Github.

At the time of writing this, two categories of programs are being developed:
* Programs that are completely written in Rust and are so-called bare-metal. That is, they are very close to hardware and do not need Rust's standard library. Like: [esp-hal](https://github.com/esp-rs/esp-hal)
* Programs that use the esp-idf API through FFI instead of implementing everything from scratch in Rust. These programs must use the Rust standard library because they use functions needed to communicate with C and esp-idf, and therefore require std. For example: [esp-idf-hal](https://github.com/esp-rs/esp-idf-hal)

Using the first category is almost impossible due to the immaturity and lack of preparation of many APIs required in industrial work. So currently, most people use the second category, which is based on the tested and advanced esp-idf framework. [[1]](#1)

# Software Packages Needed for Rust Development on ESP32
In the Rust language, a crate is the smallest unit recognized by the Rust compiler. In general, crates can be considered a software package or a project.
The following packages are used in developing Rust applications with esp-idf:
* [embedded-hall](https://github.com/esp-rs/embedded-hal): This package contains a set of programming interfaces or traits for using HAL in embedded environments and does not include any code related to a specific microcontroller.
* [esp-idf-hal](https://github.com/esp-rs/esp-idf-hal): This package is the implementation of embedded-hal for ESP32 microcontrollers through esp-idf.
* [embedded-svc](https://github.com/esp-rs/embedded-svc): This package contains a set of programming interfaces or traits for using different services such as wifi, bluetooth and httpd in embedded environments and does not include any code related to a specific microcontroller.
* [esp-idf-svc](https://github.com/esp-rs/esp-idf-svc): this package is the implementation of embedded-svc for ESP32 microcontrollers through esp-idf.
* [esp-idf-sys](https://github.com/esp-rs/esp-idf-sys): This package provides raw and insecure connections to the esp-idf library written in C.

Also, since most ESP32 microcontrollers use the Xtensa architecture and this architecture is not supported by default by the Rust compiler and its backend, which is LLVM, Espressif maintains a [fork](https://github.com/esp-rs/rust-build) of the Rust compiler that supports this architecture. To get started, this fork of the Rust compiler must be downloaded and installed.
The rest of the software needed to flash and monitor the microcontroller and other tips can be found in the [Espressif documentation](https://esp-rs.github.io/book/introduction.html) for Rust programming. There is a step-by-step tutorial on how to set up a Rust project in these documents, and it is recommended to read them. [[1]](#1)
To flash, it is enough to install the espflash program from the tutorial above and use the following command to flash the binary file placed in the project on the microcontroller:
```shell
$ espflash /dev/ttyUSB0 target/xtensa-esp32-none-elf/release/examples/blinky --monitor
```
And to create a binary file suitable for OTA use:
```shell
$ espflash save-image --partition-table partitions_singleapp.csv ESP32 target/xtensa-esp32-espidf/release/sem sem103
```

# Filesystem
In order to be able to store and manage a lot of data in the flash memory of the microcontroller, we need a file system. According to our needs, this file system should do three things:
* Be resistant to power outages. Microcontrollers that are used to monitor the power flow may be interrupted at any moment, and this sudden event should not cause the stored data structures to suffer and the system to be in an unknown state.
* RAM memory in microcontrollers is very limited, and regardless of the size of the file in the memory and the number of files, the use of RAM should be constant and not change with the increase of input.
* Flash memories have a limited number of write cycles, and if a physical part of the flash is written a large number of times (usually 10,000 to 100,000 times), that part will be damaged. Therefore, the file system must take care to use all memory blocks to spread the depreciation over all blocks.
After my research I have come to the conclusion that there are generally three file systems used for flash memory in microcontrollers. These three file systems and the NVS method, which stores values in memory as key and value pairs and is not considered a file system, have been compared together. [[2]](#2)

[SPIFFS](https://github.com/pellepl/spiffs) and [LittleFS](https://github.com/littlefs-project/littlefs) seem to be good options. Examining the repository related to SPIFFS, we find that there is not much development for this file system and the project is almost abandoned. But LittleFS shows a lot of potential.
LittleFS is constantly being developed and has a fairly large community behind it. The MicroPython project also uses this file system by default. This relatively new file system uses the Copy-on-Write method, in the simplest case, it works in such a way that when a branch is changed in the file system tree, the whole branch is copied somewhere else and then the change is applied. The previous branch is marked deleted by changing a flag that indicates it no longer has the correct value. With this method, there is no need to erase the memory unnecessarily and use flash memory writing cycles, and you can also make sure that if a problem occurs during writing or the power is cut off, because the values are copied before changing, the original values will not be tampered with. They remain valid until the write is complete. It should be noted that many optimizations are used in this file system to improve the efficiency of the COW method, the details of which are available in the project documentation. [[3]](#3)

This file system is not suitable for situations where it is necessary to change the middle of a large file, because since the entire file is copied before the change, the entire memory space may be filled due to this copy and the continuation of the file may have problems. But this problem does not exist if we just add something to the end of the file. But it is still generally better to avoid increasing the size of each file, which we will do with the sharding method.

# Measuring current and voltage using SCT sensors

These sensors are connected as a clip around the current carrying wire. The current inside the wire passes through a thick metal ring inside the clip and induces a current inside this ring. Another coil with a specific number of turns is connected to the metal ring, and when a current is induced in the big ring, another current proportional to it is induced in the coil; This ratio is determined by the number of coil turns. So if, for example, 100 amps are passed through the desired wire, depending on the number of small coil loops, the output current of the sensor can be something like 10 milliamps. [[6]](#6)
To read the output of the sensors, they are connected through a 3.5 mm jack converter to the pins of the microcontroller that have ADC capability. Since we are going to be working with city AC power, we can calculate the RMS current and voltage using readings from several sensor outputs. The point that should be noted is that the ADC output is a positive number between 0 and n depending on its resolution, and therefore it is necessary to find the middle of the sine wave corresponding to the electric current and subtract it from the whole wave to get our numbers. around 0. This number to be subtracted is known as the DC offset, which is shown in below. [[7]](#7)

<p align="center"><img src="https://user-images.githubusercontent.com/57039957/214002385-c5b7fc50-0502-45d7-8dc1-a736d5be2a68.png" width="400"></p>

The algorithm:
* First, we find the middle value of the voltage in a loop. This is easily done by having the maximum value that the ADC can output.
* Then, in another loop, we continuously read the voltage and current values until the time ends or a certain number of passes through the middle of the wave has been done, then we apply a low-pass filter, and finally collect the readings in the necessary variables. During this time, we store the minimum and maximum value read for current and voltage, and after the loop is finished, we improve the offset value, which is the middle value in the wave.
* At the end, we calculate the RMS values for voltage and current and get the real and apparent energy and kwh. You get the kwh value cumulatively; That is, when the corresponding function is called, the kwh values are added together and whenever we reach an hour, the kwh value will have the correct value for that hour.

# References
* <a id="1">[1]</a> [esp-rs book](https://esp-rs.github.io/book/introduction.html)
* <a id="2">[2]</a> [Espressif Documentations about file systems.](https://docs.espressif.com/projects/espressif-esp-iot-solution/en/latest/storage/file_system.html)
* <a id="3">[3]</a> [LittleFS Design Guide.](https://github.com/littlefs-project/littlefs/blob/master/DESIGN.md)
* <a id="4">[4]</a> [Thingsboard Documentation.](https://thingsboard.io/docs/reference/http-api/)
* <a id="5">[5]</a> [Espressif Documentation about partition tables.](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/partition-tables.html)
* <a id="6">[6]</a> [Open energy monitor guides.](https://learn.openenergymonitor.org/electricity-monitoring/ac-power-theory/arduino-maths)
* <a id="7">[7]</a> [Espressif Documentation About ADC.](https://docs.espressif.com/projects/esp-idf/en/v3.3/api-reference/peripherals/adc.html)
* <a id="8">[8]</a> [Thingsboard Documentation for Sending Telemetry Data.](https://thingsboard.io/docs/user-guide/telemetry/)
* <a id="9">[9]</a> [Sharding methods.](https://en.wikipedia.org/wiki/Shard_(database_architecture))

