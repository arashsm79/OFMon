# Offile-first Smart Energy Montoring
> Associated mobile app available [here](https://github.com/arashsm79/OFMonMobile).
* [Introduction](#Introduction)
* [Rust and ESP32](#Rust-and-ESP32)
* [Software Packages Needed for Rust Development on ESP32](#Software-Packages-Needed-for-Rust-Development-on-ESP32)
* [Filesystem](#Filesystem)
* [Measuring current and voltage using SCT sensors](#Measuring-current-and-voltage-using-SCT-sensors)
* [Using LittleFS in Rust](#Using-LittleFS-in-Rust)
* [Sharding](#Sharding)
* [Dealing with power outages](#Dealing-with-power-outages)
* [Rust program routine](#Rust-program-routine)
  * [Webserver](#Webserver)
* [Flash Memory Partitioning](#Flash-Memory-Partitioning)
* [Thingsboard Platform](#Thingsboard-Platform)
* [Thingsboard Flutter Mobile App](#Thingsboard-Flutter-Mobile-App)
  * [Scan Nearby Access Points](#Scan-Nearby-Access-Points)
  * [Initial Setup of The Device](#Initial-Setup-of-The Device)
  * [Receiving Data From The Device](#Receiving-Data-From-The-Device)
  * [ER Diagram and Database Architecture](#ER-Diagram-and-Database-Architecture)
  * [Perform OTA via Mobile](#Perform-OTA-via-Mobile)
  * [Sending Readings to The Server](#Sending-Readings-to-The Server)
* [Display Data on The Server Side](#Display-Data-on-The-Server-Side)
* [Conclusion](#Conclusion)
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

# Using LittleFS in Rust
To use LittleFS in the esp-idf environment, you can use the [esp-littlefs](https://github.com/joltwallet/esp_littlefs) project. As in normal C projects, you can add this package as a component to your project and use standard C functions to work with the file the system.
But to use esp-little in Rust, you need the help of esp-idf-sys. In the Rust project settings, we specify that when compiling the program, it should create the necessary binding to use this file system in Rust.

# Sharding
As explained earlier, using large files to write to LittleFS is not highly recommended; Therefore, instead of having a large file in the system that will be written into for months which reduces the system's performance, we can use sharding to solve this problem. In this way, a fixed size is set for each file, and if the size exceeds that limit when writing, a new file will be created and the system will write the values in that new file from then on. This method is also widely used in databases to avoid handling large files. [[9]](#9)

# Dealing with power outages
The hardware that we had at hand did not include a separate RTC module and it was not possible to make any changes to the hardware. Therefore, since the timestamps related to the readings are recorded in the device, to improve the error caused by power failure, the microcontroller periodically stores its RTC value in the flash memory and every time it starts working, the stored RTC value is read from the memory and is set as the system clock. After setting the clock, it appends the RTC value read from the memory in a file called powerloss_log.
When receiving values from the microcontroller by the mobile application, the list of power failure events is also sent, and the mobile application tries to correct the timestamp of the data as much as possible by calculating the total duration of the power failure experienced by the microcontroller.

# Rust program routine
At the beginning of the program, we launch the file system. This setup will format the LittleFS partition for the first time and only mounts it in the next times.
The Rust program is executed as a Task in the FreeRTOS operating system that esp-idf uses, and other tasks such as handling requests by the web server is done in other tasks. Therefore, since the microcontroller that I was using has more than one core, it is possible to run the main Rust code in parallel with the code related to the web server handlers. Thus,to prevent data race, a Mutex can be used for all operations that need to work with the file system. In the next step, we create a mutex with the LittleFS handle behind it.
It then finds the latest shard to store readings in and executes the RTC and power-down routines discussed earlier.
In the next step, the wifi system is set up. In this step, Micro sets up an access point; The ssid of this access point also includes the MAC address of the micro, so when several micros are together, they can still be distinguished individually.
After that, the web server and all its handlers are started and registered, and then the ADC micro system is started. Web server handlers are explained in more detail below.
In the next part, after making sure that the above steps are started, the firmware version that is currently running is confirmed. This is to ensure correct OTA update. For example, if a wrong update is done through OTA and the initial setup fails or the micro is reset due to an error, because the current version is not verified, the micro will automatically go to the previous version that worked properly.
In the final stage, the micro enters a loop that periodically reads and aggregates the values from the sensor, and stores the aggregated values in the memory after one hour.

## Webserver
After running the web server, the following handlers are registered in it:
* /telemetry: the data of all shards is sent to the requester in binary form and in http chunk format.
* /powerloss_log: All data related to power loss is sent to the requester.
* /time: The new clock is received in unix epoch time format and RTC is set with it.
* /token: if the request is a GET, the current token is sent, and if it is a POST, the sent token is stored in the current token array. The use of the token is explained below.
* /reset: All information except time is erased from the memory.
* /ota: The data related to the new version of the program is received as a chunk and placed in the next OTA partition. If the binary file is received correctly, the new partition will be set as a bootable partition in the OTA header. OTA update happens only when the received version is higher than the current version.
* /version: Sends the current version to the requester.

# Flash memory partitioning
The file below is given as a partition table to the software that flashes the program to create the partitions in the flash memory:
```config
# Name, Type, SubType, Offset, Size, Size in Bytes
nvs, data, nvs, 0x9000, 0x4000, 16k
otadata, data, ota, 0xd000, 0x2000, 8k
phy_init, data, phy, 0xf000, 0x1000, 4k
ota_0, app, ota_0, 0x10000, 1M, 1M
ota_1, app, ota_1, , 1M, 1M
littlefs, data, spiffs, , 0x1e0000,1.9M
```
As you can see, two partitions are considered for OTA; Each time a new program is downloaded as a binary file, one of the OTA partitions is used. The otadata partition specifies which partition the bootloader should run from during boot.
nvs partition is used for wifi and phy_init partition is used for physical layer and radio. Finally, the littlefs partition corresponds to the littlefs file system where data is stored. [[5]](#5)

# Thingsboard platform
Thingsboard is one of the most famous Internet of Things platforms that is completely open source and is used all over the world. Installation and commissioning of Thingsboard server was done through docker on a VM created in proxmox.
Thingsboard can be used for:
* Device management
* Receive and store data that devices send
* Process data from devices and perform various tasks based on that data, such as sending an email or running a piece of code.
* Displaying data in different graphs
* Creating and launching firmware updates through OTA
* Creating different user accounts with different access levels
* And …
Devices in this platform have names and descriptions, and each device is given a unique access token when it is created. Devices can send their data directly to the server with their token to the following endpoint in a specified format:
```
http://<server-address>/api/v1/<token>/telemetry
```
Each device is also located in a profile; Profiles are used to separate and group devices that perform the same task and must be managed together. For example, when using OTA update, we can present a new version of the binary file uploaded to the server to all devices that are in a profile, or when drawing a graph, we can display the data of all devices that are in a specific profile. [[4]](#4)[[8]](#8)

# Thingsboard Flutter mobile app
Thingsboard also has a mobile app written with the flutter framework called [thingsboard_flutter](https://github.com/thingsboard/flutter_thingsboard_app), which allows users to view and interact with their dashboards and see graphs of device data. One of the things that was done, and we will talk about it further, is to fork this program and add the ability to collect offline data from devices and send it to the server. In the figure below, you can see a view of the added tab called collect:
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/214007538-20991e23-d3e7-49b3-83df-9bf09bc33e3e.png" height="400"></p>

# Scan nearby access points
The [wifi_iot](https://github.com/flutternetwork/WiFiFlutter/tree/master/packages/wifi_iot) library is used to scan and find nearby devices. This library can return the list of all nearby APs and connect to the requested AP by giving it an ssid. By default, the ssid of all devices with `SEM-` in the beginning is displayed, and therefore it is easy to display only the APs related to the required devices from the list of all nearby APs. Finally, the user can connect to the desired AP by clicking on it.

# Initial setup of the device
After connecting to the device's AP and before doing anything else, three things need to be done with the device:
* The token related to the device representation on the server should be given to the device. With this, every time the mobile application wants to get data from a device, it also gets its token and stores it in its database. To copy the token, a section has been added in the Devices tab through which users can click on the desired device and copy the corresponding token. After copying the token, it can be pasted in the field specified during the initial startup of the device.
* Send the current time in unix epoch time format to the device to update its RTC. If you are connected to the Internet, this time is taken online, and otherwise, the time of the mobile phone itself is used.
* The memory of the device should be reset to make sure that no unwanted data remains in the memory.
Finally, a record is created in the database for this device that associates the ssid of the AP we are connected to to the device token. Also, the timestamp of the last connection to this ssid is also recorded in this record. With this, for example, when scanning, we can separate all the devices that we have connected to in the last 10 minutes with a different color.

# Receiving data from the device
To receive information from the device, the following procedure is performed:
* First, the device token is taken.
* Then all the information about the readings is taken from the device.
* The updated time is sent to the device.
* A list of all power outage events is taken.
* Binary readings are converted to their corresponding objects.
* The difference between the time of the last data sent and the current time indicates the amount of power outage. If this amount is more than one hour, we divide it by the number of power outage events; The obtained number should be placed in the places where there is a power outage, which we obtain using the power outage event.
* Finally, the readings are stored in the mobile database and a request is sent to reset the device's memory.


# ER diagram and database architecture
The ER diagram of the database used in mobile is as follows:
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/214009672-edab321e-04da-4634-a992-9c27f8d1c14e.png" height="400"></p>

As you can see, all the devices are placed in the devices table. The token given to each device is a combination of the access token and its device profile id on the thingsboard server. So after connecting to a device, its token is first taken and placed in this table along with its ssid. This feature has been added that after connecting to the Internet and entering the All Devices section of the mobile app, the name assigned to each device on the server is added to this database; This is actually a link between a device's ssid and its name on the server, which can be used to display each device's name next to it when scanning for APs.
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/214009684-6d126f7b-7145-4ccc-9d2c-2a8cf6b584a9.png" height="400"></p>

The last_checked field shows the time of the last time the mobile phone was connected to the device. If this time is less than ten minutes ago, when scanning for APs, the corresponding AP of the device will turn green to make it easier for the user to identify which device to connect to in the next step.


# Perform OTA via mobile
The ota table is for managing otas and their binary files. Every time the user enters the Devices tab, the mobile application automatically checks through the server whether there is a new OTA update for the devices registered in the mobile database, and if there is, it downloads it.
OTA updates work based on device profile, and when checking for OTA, it is checked that the version on the server is greater than the version stored on the device, and if it is, the new version is downloaded and replaces the previous version.
When connecting to the device, the program version of the device and its device profile id are taken first, and if there is a newer update file for it, the user can update the device by selecting the Update Device Firmware option.
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/214009698-a229f132-d584-4c25-af40-73bdd9fefcb8.png" height="400"></p>


# Sending readings to the server
By selecting the Sync With Server option, the user can send readings related to any device stored in their database to the server. This is possible because in the mobile database, all access tokens of the devices are also stored, and as mentioned earlier, to send the readings to the server, it is enough to send the data in the specified format to the following address:
```
http://<server-address>/api/v1/<token>/telemetry
```

# Display data on the server side
To display data on the thingsboard server, we need to design a dashboard so that the graph of all the devices that are in the SEM profile can be displayed on this dashboard. The designed charts are:
* Pie chart to compare total energy consumption of devices
* Three graphs to display kwh in time intervals: hourly, daily and monthly
* Graph of average hourly current and voltage consumption (the voltage value is constant)
Below is an image of the designed dashboard:
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/214009791-8c2685f9-d132-4330-99fe-4d2d34b6bbec.png" height="400"></p>
<p align="center"><img src="https://user-images.githubusercontent.com/57039957/214009813-76c19864-3214-4307-94a4-2f40a28cca59.png" height="400"></p>


# Conclusion
I have implemented an Internet of Things platform for power consumption monitoring based on ESP32 processors and Thingsboard server. One of the main problems that current IoT systems have is the need to constantly connect to a server or gateway; My method solves this problem by means of mobile application and using the AP capability of ESP32 microcontrollers with emphasis on offline operation. In this way, the mobile application acts as a gateway that connects to the devices and receives their data. In the future, an autopilot mode can be created for the mobile app to collect data from devices automatically; Of course, this is not possible in the Android operating system because applications do not have the right to do this without notifying the user and approving the application.

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

