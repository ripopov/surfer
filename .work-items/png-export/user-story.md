---
inclusion: manual
---

# User Story: Export Waveform as PNG

## User Persona Definition

- **Name:** CI/CD Engineer Carl
- **Description:** Carl is a software engineer responsible for maintaining and improving the continuous integration and continuous delivery pipelines for hardware description language (HDL) projects. He needs to quickly verify changes in waveform files and integrate visual diffing into his automated workflows to catch regressions early.

- **Name:** GUI User Gina
- **Description:** Gina is a hardware design engineer who uses Surfer interactively to debug and analyze waveforms. She occasionally needs to capture screenshots of specific waveform views for documentation, presentations, or sharing with colleagues.

## User Story Format

- **As a** CI/CD Engineer Carl
- **I want to** specify commands within a Surfer command file to set zoom for specific viewports and export specific views or multiple regions of a waveform as PNG images
- **so that** I can automate focused visual diffs of waveform files in merge requests, concentrating on signals of interest and quickly identifying changes without excessive image data, all while maintaining a consistent and reproducible setup through command files.

- **As a** GUI User Gina
- **I want to** select a menu item to export the current waveform plot as a PNG image
- **so that** I can easily capture and share visual representations of my waveform analysis for documentation or collaboration.

## Acceptance Criteria Requirements

- WHEN I execute a Surfer command file containing a `set_zoom` command with a `VIEWPORT_ID`, THEN Surfer SHALL adjust the zoom for that specific viewport.
- WHEN I execute a Surfer command file containing an `export_png` command with parameters defining a specific view or multiple regions of interest and a `.png` file path, THEN Surfer SHALL generate one or more PNG images of the specified waveform views at the specified path(s). This includes options to export a single specified viewport, or by default, all viewports either as a single combined PNG or as separate PNGs.
- WHEN the export is successful, THEN the command file execution SHALL complete without errors.
- WHEN the export fails (e.g., invalid path, rendering error, invalid region specification), THEN the command file execution SHALL terminate with an error and provide an informative error message.
- WHEN exporting via command file, THEN the generated PNG image(s) SHALL accurately represent the visible waveform data within the specified view(s) or region(s), including all displayed signals, cursors, and markers relevant to that region.

- WHEN I select the "Export Plot as PNG..." menu item in the Surfer GUI, THEN a file dialog SHALL appear, allowing me to choose the output path and filename for the PNG image.
- WHEN I confirm the file dialog, THEN Surfer SHALL export the current waveform plot as a PNG image to the specified location.
- WHEN the GUI export is successful, THEN a confirmation message (e.g., a toast notification) SHALL be displayed.
- WHEN the GUI export fails, THEN an informative error message SHALL be displayed to the user.
- WHEN exporting via GUI, THEN the generated PNG image SHALL accurately represent the currently displayed waveform data, including all visible signals, cursors, and markers.

## Value Proposition

This feature will enable automated, focused visual regression testing for waveform changes, significantly reducing manual review time and improving the reliability of HDL development workflows by allowing precise targeting of visual diffs and reproducible export configurations through command files. Additionally, it will enhance the interactive user experience by providing a convenient way for GUI users to capture and share waveform plots.

## Success Metrics

- **Primary Metric**: Successful generation of PNG images for specified views/regions via `export_png` commands within Surfer command files in automated CI pipelines, and successful interactive export via the GUI menu item.
- **Secondary Metrics**: Reduction in time spent on manual waveform review; increased confidence in waveform changes due to automated visual diffs; efficient storage and processing of visual diff data due to focused exports; improved reproducibility of waveform views through command files; increased user satisfaction for interactive waveform capture.
