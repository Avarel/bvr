# bvr

![BVR CLI](assets/simple.png)

Powerful pager written in rust, purpose-built for chewing through logs.

BVR (pronounced "beaver") is still under heavy development.

## Goals
* Fast and responsive
* Intuitive and easy to use
* Targeted use: scrolling through log files

## Non-Goals
* Syntax highlighting
* Editing files
* 

## Features
| Feature              | Description                                                 | Progress |
| -------------------- | ----------------------------------------------------------- | -------- |
| Better Mouse Support | Use mouse to interact with the TUI.                         | Planned  |
| Custom Keybindings   | Customize the keybindings of the program.                   | Planned  |
| Search Filters       | Select and disable additive search filters.                 | Basic    |
| Searching            | Search regex in the file.                                   | Basic    |
| Multiplexing         | View multiple files through tabs or windows.                | Basic    |
| Piping Files         | View piped outputs of other programs, ie. `cat file \| bvr` | Basic    |
| Status Bar           | View current state of the pager.                            | Basic    |
| Commands             | Use modal commands to interact with the pager.              | Basic    |
| Mouse Scrolling      | Use mouse scrolling to pan through the current viewer.      | Done     |

