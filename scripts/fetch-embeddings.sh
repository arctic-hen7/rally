#!/bin/sh
set -eu
dest=${1:?Supply the model directory}
mkdir -p "$dest"
base=https://huggingface.co/minishlab/potion-base-8M/resolve/bf8b056651a2c21b8d2565580b8569da283cab23
for file in config.json tokenizer.json model.safetensors; do
    curl --fail --location --silent --show-error --retry 3 "$base/$file" --output "$dest/$file.part"
    mv "$dest/$file.part" "$dest/$file"
done
