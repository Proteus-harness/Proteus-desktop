# Noto Color Emoji

Unmodified upstream color bitmap font, embedded once in the native executable.
The existing Swash renderer supports its CBDT/CBLC glyphs. No system font or
fontconfig configuration is changed. Its SIL Open Font License 1.1 is in
`OFL.txt` and accompanies the portable distribution's third-party notices.

- Project: <https://github.com/googlefonts/noto-emoji>
- Commit: `1ffdd21391dd1f25c081fa93a9dea0c7c029442b`
- Font: <https://raw.githubusercontent.com/googlefonts/noto-emoji/1ffdd21391dd1f25c081fa93a9dea0c7c029442b/2D/fonts/NotoColorEmoji.ttf>
- License: <https://raw.githubusercontent.com/googlefonts/noto-emoji/1ffdd21391dd1f25c081fa93a9dea0c7c029442b/2D/fonts/LICENSE>
- Font size: 10,730,124 bytes
- Font SHA-256: `15671215ab769fdc7162a045d56fd7d7e477c51b04e6b3c761d914d8fdd6cc44`

Fedora's newer Noto COLRv1 face has valid glyph advances but requires paint
graphs unsupported by Swash 0.2. After the compatible bitmap face is registered,
the native text backend excludes conflicting COLRv1-only Noto faces from its
own font database. Other installed families and supported faces remain intact.
The bytes are borrowed from the executable; glyphs use the existing renderer
and atlas, without a second application-level bitmap cache.
