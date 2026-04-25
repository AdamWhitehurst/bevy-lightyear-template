# Task: Palette-Based Coloring for Sprite Rigs

Add palette-driven recoloring to the sprite rig system so that rigged sprite bones are tinted/colored by a shader according to indices defined on a new `SpriteRigPalette` component. The component holds the set of colors for a rig instance; the shader maps per-pixel index data in the bone's sprite texture to a color drawn from the palette.
