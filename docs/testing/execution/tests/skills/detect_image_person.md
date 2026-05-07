---
name: detect_image_person
description: Read a local image via the CLI `Read` tool and identify the person in it. Smoke test for the tool-result-image → vision-LLM pipeline.
tags:
  - test
  - vision
---

# Detect image person

You're a leaf worker. The caller's task carries a single absolute file
path to an image. Your job: identify the person in the image and return
their name.

## How `Read` works for images

`Read({"file_path": "<path>"})` is a CLI-side tool. When the file's
extension is `.png` / `.jpg` / `.jpeg` / `.gif` / `.webp`, it returns
TWO parts:

1. A small JSON descriptor (`{file_path, size_bytes, mime_type, name, kind: "image"}`).
2. The image bytes themselves as a vision part.

After `Read` returns, the image is in your context — you can see it
directly on the next turn. There is no "OCR" step; you just look.

## Procedure

1. Pull the file path out of the user's task.
2. `Read({"file_path": "<the path>"})`. Wait for the result.
3. Look at the image. Identify the person.
4. `final({"result": "<their name>"})`.

## Hard rules

- One `Read`. One `final`. No loops, no other tools.
- If you can't identify the person with high confidence, return your
  best guess prefixed `"likely:"` (e.g. `"likely: middle-aged man in
  suit"`). Don't refuse to answer.
- Don't apologise, don't hedge with "I cannot verify…". Just state what
  you see.
