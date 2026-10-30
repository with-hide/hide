// Every System part in web/src/components/ui and the states the gallery draws
// for it. The gallery types each section's renderers from this list, so
// TypeScript refuses a state the page does not render. Keep it a plain literal.

export const GALLERY = {
  "Button": ["Default", "Default Hover", "Default Focus", "Default Disabled", "Pending", "Secondary", "Secondary Hover", "Outline", "Ghost", "Ghost Hover", "Destructive", "Destructive Hover", "Link", "Text", "Text Hover", "Text Link", "Text Link Hover", "Small", "Large", "Icon", "Icon Small", "Icon Extra Small", "Row Action", "Row Action Hover"],
  "Input": ["Default", "Placeholder", "Filled", "Focus", "Invalid", "Disabled", "Mono"],
  "Textarea": ["Default", "Placeholder", "Focus", "Invalid", "Disabled", "Bare"],
  "Row Button": ["Default", "Hover", "Focus", "Disabled", "Overlay", "Overlay Hover"],
  "Select": ["Default", "Placeholder", "Focus", "Disabled", "Open"],
  "Checkbox": ["Unchecked", "Checked", "Indeterminate", "Focus", "Disabled", "Checked Disabled"],
  "Switch": ["Off", "On", "Focus", "Disabled"],
  "Radio Group": ["Default", "Focus", "Disabled"],
  "Toggle Group": ["Default", "Hover", "Focus", "Disabled", "Extra Small"],
  "Slider": ["Default", "Focus", "Disabled"],
  "Tabs": ["Default", "Hover", "Focus", "Disabled"],
  "Badge": ["Default", "Secondary", "Destructive", "Outline"],
  "Kbd": ["Default", "Group"],
  "Separator": ["Horizontal", "Vertical"],
  "Dropdown Menu": ["Closed", "Open", "Highlighted", "Disabled Item", "Destructive Item"],
  "Context Menu": ["Open", "Highlighted", "Disabled Item"],
  "Dialog": ["Open", "With Close Button"],
  "Alert Dialog": ["Open", "Pending"],
  "Sheet": ["Right", "Left"],
  "Popover": ["Open"],
  "Tooltip": ["Open", "With Shortcut"],
  "Command": ["Default", "Filtered", "Empty"],
  "Sonner": ["Default", "With Description", "With Action"]
} as const;

export type Section = keyof typeof GALLERY;
export type StateOf<S extends Section> = (typeof GALLERY)[S][number];
