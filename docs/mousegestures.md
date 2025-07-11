# Mouse gestures

Surfer supports mouse gestured. These are activated using the middle mouse button, or if the middle mouse button is not available, but pressing Ctrl (Cmd on MacOS) and using the primary mouse button.

If the mouse pointer is close to the location where it was pressed, a graphical overlay showing the different gestures is shown.

## Configuration

It is possible to modify how the mouse gestures behave.

``` toml
[gesture]
# Size of the square encapsulating the instructions
size = 300
# Squared minimum move for the instructions to show up
deadzone = 20
# Radius, relative to size, for the background circle
background_radius = 1.35
# Gamma for the background circle
background_gamma = 0.75

# Mapping of different locations
[gesture.mapping]
north = "Cancel"
south = "Cancel"
west = "ZoomIn"
east = "ZoomIn"
northeast = "ZoomOut"
northwest = "ZoomToFit"
southeast = "GoToEnd"
southwest = "GoToStart"
```

The currently available mouse gestures
 actions are

| Name | Description |
|-----|-----|
| Cancel | No operation |
| GoToEnd | Scroll view to last time in simulation |
| GoToStart | Scroll view to first time in simulation |
| ZoomIn | Zoom in to the range defined by the start and end time of the gesture |
| ZoomOut | Zoom out a constant factor |
| ZoomToFit | Zoom to cover the whole range of the simulation |

Note that although any action can be mapped to any direction, it may not make sense to map ZoomIn to anything else that East or West, as the difference in x-direction is what determines the zoom factor.

## Measure time

It is possible to measure the time by holding shift and pressing the primary mouse button.

The behavior can be configured using the config `primary_button_drag_behavior` which can take the value `Cursor` (press shift to measure) or `Measure` (no need to press shift). There is also a preference setting for this in the Preference menu.
