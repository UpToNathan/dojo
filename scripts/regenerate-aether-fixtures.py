#!/usr/bin/env python3
"""Explicit, networked regeneration of fixtures using Aether's pinned Go source."""

import pathlib
import shutil
import subprocess
import tempfile
import urllib.request

REVISION = "29d07b1a8f474c77bf02aece948373f40abed105"
ROOT = pathlib.Path(__file__).resolve().parents[1]

COLOR_FILES = ["types.go", "convert.go", "oklab.go", "accessibility.go"]
EXTRACTION_FILES = [
    "analysis.go", "constants.go", "extractor.go", "median_cut.go", "normalize.go",
    "palette_chromatic.go", "palette_modes.go", "palette_monochrome.go",
    "palette_moods.go", "palette_practical.go", "palette_schemes.go", "pixel_sampler.go",
]


def generate():
    temporary_root = pathlib.Path("/tmp/opencode")
    temporary_root.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="aether-fixtures-", dir=temporary_root) as directory:
        source = pathlib.Path(directory)
        for package, files in [("color", COLOR_FILES), ("extraction", EXTRACTION_FILES)]:
            destination = source / "internal" / package
            destination.mkdir(parents=True)
            for filename in files:
                url = f"https://raw.githubusercontent.com/omacom/aether/{REVISION}/internal/{package}/{filename}"
                with urllib.request.urlopen(url, timeout=30) as response:
                    (destination / filename).write_bytes(response.read())
        (source / "go.mod").write_text("module aether\n\ngo 1.25\n\nrequire golang.org/x/image v0.41.0\n")
        shutil.copyfile(ROOT / "tests/reference/aether/generate.go", source / "main.go")
        (source / "internal/wallpaper").mkdir()
        (source / "internal/wallpaper/decode.go").write_text('''package wallpaper
import ("image"; "io"; _ "image/jpeg"; _ "image/png")
func DecodeImage(r io.Reader) (image.Image,error) { img,_,err:=image.Decode(r); return img,err }
''')
        (source / "internal/extraction/cache_stub.go").write_text('''package extraction
func GetCacheKey(path string,light bool) string {return ""}
func buildCacheKey(base,mode string) string {return ""}
func LoadCachedPalette(key string) ([16]string,bool) {return [16]string{},false}
func SavePaletteToCache(key string,palette [16]string) {}
''')
        subprocess.run(["go", "mod", "tidy"], cwd=source, check=True)

        def run(arguments, output):
            with output.open("wb") as stream:
                subprocess.run(["go", "run", ".", *map(str, arguments)], cwd=source, stdout=stream, check=True)

        fixtures = ROOT / "tests/fixtures"
        fixtures.mkdir(exist_ok=True)
        run([fixtures / "astro.jpg"], fixtures / "aether-palettes.json")
        run(["--sample", fixtures / "astro.jpg"], fixtures / "astro-go-samples.rgb")
        subprocess.run(["go", "run", ".", "--make-sampling-fixture", str(fixtures / "aether-sampling.png")], cwd=source, check=True)
        run(["--sample", fixtures / "aether-sampling.png"], fixtures / "aether-sampling.rgb")


if __name__ == "__main__":
    generate()
