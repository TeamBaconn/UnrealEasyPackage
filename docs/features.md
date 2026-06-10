# Features - a screen-by-screen tour

A walk through what UnrealEasyPackage does and the actions on each screen. Every parameter in the app is labeled and self-explaining, so this focuses on the main features rather than each individual setting.

← back to the [README](../README.md)

## Build & run

![Build tab](../screenshots/build-history.jpg)

The home of a project. Pick a saved profile from the dropdown and hit **Run**, or open the profile editor with the wrench. Below sits the full **build history** - filter by platform, config, target, or status, page through past runs, and open the output folder for any build.

## Build profiles

![Build settings](../screenshots/build-settings.jpg)

Profiles capture a whole package configuration: platform, target, and which **phases** run (Build, Cook, Stage, Pak, Archive, Copy Extras). Toggle phases on or off, add file mappings for extras like `steam_appid.txt`, and save. Create as many profiles as you have build flavors.

## Live pipeline + logs

![Build logs](../screenshots/build-logs.jpg)

While a build runs, each phase shows up as a node in a **pipeline graph** (done / running / pending / failed) with per-phase timing. The streaming **console** below tints lines by severity, counts warnings and errors, and is searchable. The exact command that was launched is shown at the bottom. Cancel at any time.

## Dashboard

![Dashboard](../screenshots/dashboard.jpg)

Trends across your builds for a chosen platform / target / configuration: latest build size and duration (with deltas), success rate, total builds, and charts for **build size over time**, **warnings & errors**, and cumulative **pass vs fail**.

## Footprint cleanup

![Clean tab](../screenshots/clean-footprint.jpg)

A categorized scan of everything a build scatters across the project - staged builds, cooked content, shader caches, binaries, and per-target intermediates - with sizes and a reclaimable total. Tick the categories you want gone and delete them in one action. Safe-to-delete vs risky items are separated so you don't nuke something you need.

## Plugin packaging

![Package plugin](../screenshots/plugin-packaging.jpg)

Open a `.uplugin` instead of a project and you get a focused **Package Plugin** tool: pick the engine, set an output folder and a templated folder name, and run `BuildPlugin -rocket`. Optionally strip `Binaries` and `Intermediate` afterward so the output is ready for FAB submission. Live log streaming, same as a project build.

## Project tools

![Project tools](../screenshots/project-tools.jpg)

Run engine commandlets without leaving the app: **Resave Assets** (bake in Core Redirects, fix up redirectors, re-serialize Blueprints) and **Validate Assets** (run Data Validation across the project). Output streams to the run log just like a build.
