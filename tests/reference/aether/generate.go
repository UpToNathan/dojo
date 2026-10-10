// Fixture harness for the unmodified Aether algorithm at the revision pinned by
// scripts/regenerate-aether-fixtures.py. This does not launch or apply Aether.
package main

import (
	"aether/internal/extraction"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"log"
	"os"
)

var modes = []string{"normal", "monochromatic", "analogous", "pastel", "material", "colorful", "muted", "bright", "complementary", "triadic", "split-complementary", "tetradic", "fire", "ocean", "forest", "earthtone", "neon", "sunset", "vaporwave", "midnight", "aurora", "high-contrast", "duotone"}

type input struct {
	Name   string   `json:"name"`
	Colors []string `json:"colors"`
	Counts []int    `json:"counts"`
}
type expected struct {
	Mode   string     `json:"mode"`
	Light  bool       `json:"light"`
	Colors [16]string `json:"colors"`
}
type fixture struct {
	Input    input      `json:"input"`
	Expected []expected `json:"expected"`
}

func main() {
	if err := run(); err != nil {
		log.Fatal(err)
	}
}

func run() error {
	if len(os.Args) > 2 && os.Args[1] == "--make-sampling-fixture" {
		img := image.NewNRGBA(image.Rect(0, 0, 73, 37))
		for y := 0; y < 37; y++ {
			for x := 0; x < 73; x++ {
				img.SetNRGBA(x, y, color.NRGBA{byte(x*17 + y*3), byte(x*7 + y*13), byte(x*11 + y*19), byte((x/9 + y/7) % 4 * 85)})
			}
		}
		f, err := os.Create(os.Args[2])
		if err != nil {
			return err
		}
		if err := png.Encode(f, img); err != nil {
			f.Close()
			return err
		}
		return f.Close()
	}
	if len(os.Args) > 2 && os.Args[1] == "--sample" {
		pixels, err := extraction.LoadAndSamplePixels(os.Args[2])
		if err != nil {
			return err
		}
		raw := make([]byte, 0, len(pixels)*3)
		for _, p := range pixels {
			raw = append(raw, byte(p.R), byte(p.G), byte(p.B))
		}
		_, err = os.Stdout.Write(raw)
		return err
	}
	cases := []input{
		{"chromatic", []string{"#101322", "#dddddd", "#d04555", "#6cb077", "#e1c777", "#5688dc", "#b965bd", "#5bb9c4", "#25415b", "#88746c", "#3a4c34", "#677a95"}, []int{5000, 2000, 500, 700, 300, 1000, 450, 800, 600, 100, 200, 300}},
		{"gray", []string{"#000000", "#121212", "#333333", "#444444", "#777777", "#888888", "#aaaaaa", "#bbbbbb", "#cccccc", "#eeeeee", "#ffffff", "#555555"}, []int{800, 700, 600, 500, 400, 300, 200, 100, 80, 60, 40, 20}},
		{"tinted", []string{"#0e1620", "#1d3046", "#304763", "#456888", "#68849f", "#9ab0cb", "#b8c4d2", "#d4dbe2", "#35455b", "#567493", "#7394b1", "#dae6f2"}, []int{800, 700, 600, 500, 400, 300, 200, 100, 80, 60, 40, 20}},
		{"warm", []string{"#211009", "#462515", "#78523c", "#9f764c", "#bb9649", "#d4a166", "#ecd1b1", "#f4e2cf", "#633323", "#a05243", "#cc6841", "#f5b494"}, []int{800, 700, 600, 500, 400, 300, 200, 100, 80, 60, 40, 20}},
	}
	if len(os.Args) > 1 {
		colors, counts, err := extraction.ExtractDominantColors(os.Args[1], extraction.DominantColorsToExtract)
		if err != nil {
			return err
		}
		cases = append(cases, input{"astro", colors, counts})
	}
	var output []fixture
	for _, in := range cases {
		total := 0
		for _, count := range in.Counts {
			total += count
		}
		weights := make([]float64, len(in.Counts))
		for i, count := range in.Counts {
			weights[i] = float64(count) / float64(total)
		}
		result := fixture{Input: in}
		for _, mode := range modes {
			for _, light := range []bool{false, true} {
				colors := extraction.NormalizeBrightness(extraction.GeneratePaletteByMode(in.Colors, weights, light, mode))
				result.Expected = append(result.Expected, expected{mode, light, colors})
			}
		}
		output = append(output, result)
	}
	encoded, err := json.MarshalIndent(output, "", "  ")
	if err != nil {
		return err
	}
	_, err = fmt.Println(string(encoded))
	return err
}
