# -*- coding: utf-8 -*-
"""生成各扩展名的文档图标（assets/ext-icons/<ext>.ico）。

设计：白色文档纸 + 右上折角（类别色）+ 底部类别色条带印格式名（>=32px）。
类别：图像=粉 / 3D 模型=蓝 / 3D 打印=绿。4x 超采样后缩小，边缘平滑。

用法：python gen_icons.py   （需 Pillow；输出 33 个 .ico）
"""

import io
import os
import struct

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
FONT_PATH = r"C:\Windows\Fonts\segoeuib.ttf"  # Segoe UI Bold，Windows 原生观感

SIZES = [16, 24, 32, 48, 64, 256]
SS = 4  # 超采样倍数

CATEGORY = {
    "image": ("图像", (232, 98, 155)),   # 粉
    "model": ("模型", (61, 139, 224)),   # 蓝
    "print": ("打印", (52, 176, 122)),   # 绿
}

EXT_CATEGORY = {
    "png": "image", "jpg": "image", "jpeg": "image", "bmp": "image",
    "webp": "image", "tiff": "image", "tif": "image", "gif": "image", "ico": "image",
    "obj": "model", "stl": "model", "gltf": "model", "glb": "model", "ply": "model",
    "off": "model", "3ds": "model", "dae": "model", "vtk": "model", "wrl": "model",
    "vrml": "model", "vrm": "model", "fbx": "model", "pmx": "model", "pmd": "model",
    "usd": "model", "usda": "model", "usdc": "model", "usdz": "model",
    "amf": "print", "3mf": "print", "gcode": "print", "gco": "print", "g": "print",
}


def draw_one(ext, s):
    """画一张 s x s 的文档图标，返回 RGBA Image。内部按 SS 倍超采样。"""
    cat = EXT_CATEGORY[ext]
    accent = CATEGORY[cat][1]
    label = ext.upper()

    S = s * SS
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # 文档纸矩形（留边距，右上角折角）
    mx = round(S * 0.10)
    mt = round(S * 0.05)
    mb = round(S * 0.07)
    px1, py1, px2, py2 = mx, mt, S - mx, S - mb
    fold = round(S * 0.24)  # 折角直角边长

    r = max(2 * SS, round(S * 0.04))

    # 纸体：用 path 挖掉右上角
    d.polygon(
        [(px1, py1 + r), (px1 + r, py1), (px2 - fold, py1),
         (px2, py1 + fold), (px2, py2 - r), (px2 - r, py2),
         (px1 + r, py2), (px1, py2 - r)],
        fill=(255, 255, 255, 255),
    )
    # 纸体描边（不含折角斜边）
    line_w = max(SS, round(S * 0.02))
    d.line([(px1, py1 + r), (px1 + r, py1), (px2 - fold, py1)], fill=(196, 196, 200, 255), width=line_w)
    d.line([(px2, py1 + fold), (px2, py2 - r), (px2 - r, py2), (px1 + r, py2), (px1, py2 - r), (px1, py1 + r)],
           fill=(196, 196, 200, 255), width=line_w)

    # 折角三角
    d.polygon([(px2 - fold, py1), (px2, py1 + fold), (px2 - fold, py1 + fold)],
              fill=accent + (255,))
    # 折角描边（斜边）
    d.line([(px2 - fold, py1), (px2, py1 + fold)], fill=accent + (255,), width=line_w)

    # 底部类别色条带
    band_h = round(S * 0.30)
    band_b = round(S * 0.10)
    bx1, by1, bx2, by2 = px1, py2 - band_b - band_h, px2, py2 - band_b
    d.rectangle([bx1, by1, bx2, by2], fill=accent + (255,))

    # 条带上的格式名（小尺寸不画字，只留色带）
    if s >= 32:
        text = label
        max_w = int((bx2 - bx1) * 0.88)
        size = int((by2 - by1) * 0.70)
        while size > 4:
            font = ImageFont.truetype(FONT_PATH, size)
            tw = d.textlength(text, font=font)
            if tw <= max_w:
                break
            size = int(size * 0.9)
        th = size
        tx = (bx1 + bx2 - tw) / 2
        ty = (by1 + by2 - th) / 2 - size * 0.08
        d.text((tx, ty), text, font=font, fill=(255, 255, 255, 255))

    # 压缩噪声：给纸面加两条淡灰"文本行"装饰（仅大尺寸）
    if s >= 48:
        line_y = py1 + fold + round(S * 0.06)
        for i in range(2):
            ly = line_y + i * round(S * 0.09)
            d.line([(px1 + round(S * 0.09), ly), (px2 - round(S * 0.09), ly)],
                   fill=(224, 226, 230, 255), width=max(SS, round(S * 0.018)))

    return img.resize((s, s), Image.LANCZOS)


def pack_ico(images):
    """images: [(size, png_bytes)] -> ICO bytes（PNG 压缩帧）"""
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    body = b""
    for size, data in images:
        b = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", b, b, 0, 0, 1, 32, len(data), offset)
        body += data
        offset += len(data)
    return out + entries + body


def main():
    exts = sorted(EXT_CATEGORY.keys())
    total = 0
    for ext in exts:
        frames = []
        for s in SIZES:
            im = draw_one(ext, s)
            buf = io.BytesIO()
            im.save(buf, "PNG")
            frames.append((s, buf.getvalue()))
        data = pack_ico(frames)
        path = os.path.join(HERE, f"{ext}.ico")
        with open(path, "wb") as f:
            f.write(data)
        total += len(data)
        print(f"  {ext}.ico  {len(data):6d} B")
    print(f"done: {len(exts)} icons, {total} bytes total")


if __name__ == "__main__":
    main()
