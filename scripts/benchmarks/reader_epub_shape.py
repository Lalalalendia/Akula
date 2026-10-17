#!/usr/bin/env python3
"""Generate original, copyright-free EPUB layouts with identical reading text."""
from __future__ import annotations

import argparse
import random
from pathlib import Path
from xml.etree import ElementTree
from zipfile import ZIP_DEFLATED, ZIP_STORED, ZipFile

LINES = 5324
CHAPTERS = 18
WORD_POOL = ("тестовый документ состоит из вымышленных предложений проверка страниц переносы строк "
             "навигация пользователь окно колонка значение результат скорость текст функция память "
             "ширина раскладка интерфейс повторение чтение узел глава заголовок секция "
             "символ символы пробел символов фрагмент русский кириллица строка граница "
             "формат контейнер структура заметка библиотека алгоритм реализация длительность "
             "размер величина обработка память производительность измерение независимый "
             "страница временный инженер пример диагностический фокус запись индекс "
             "множество переполнение вычисление последовательность окружение загрузка "
             "состояние таблица массив разметка надежность совместимость движение указатель "
             "переход содержимое проход ошибка описание объединение контроль копирование "
             "возврат параллельность порядок повторяемость данные подход простота путь "
             "проверяемый результат характеристика пользовательский модель тип тестирование "
             "верстальщик публикация раздел загрузчик точность объект предложение").split()
_rng = random.Random(20261018)


def synthetic_line(index: int) -> str:
    words = [f'~{index:05}~']
    while len(' '.join(words).encode('utf-8')) < 225:
        words.append(_rng.choice(WORD_POOL))
    return ' '.join(words) + '. '


LINE_TEXTS = [synthetic_line(i) for i in range(LINES)]
ROOT = '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Диагностическая книга</title></head><body>{}</body></html>'
CONTAINER = ('<?xml version="1.0"?><container version="1.0" '
             'xmlns="urn:oasis:names:tc:opendocument:xmlns:container">'
             '<rootfiles><rootfile full-path="OPS/content.opf" '
             'media-type="application/oebps-package+xml"/></rootfiles></container>')


# Extracted *shape* of the user-supplied book, not its words.
# Ten of the first sixteen blocks exceed ~64 KiB at 225 UTF-8 bytes/line.
# The actual book also contains much smaller text blocks between titles.
GROUP_LINES = [500, 7, 470, 430, 6, 410, 400, 5, 395, 390, 4,
               350, 320, 4, 290, 270, 4, 255, 240, 4, 225, 220, 125]
assert len(GROUP_LINES) == 23 and sum(GROUP_LINES) == LINES


def chapters(kind: str) -> list[str]:
    groups = []
    first = 0
    for count in GROUP_LINES:
        group = ['<div>']
        for number in range(first, first + count):
            line = LINE_TEXTS[number]
            group.append(f'<p>{line}</p>' if kind == 'single-p' else f'{line}<br/>')
        group.append('</div>')
        groups.append(''.join(group))
        first += count
    assert first == LINES
    all_chapters = []
    offset = 0
    for chapter in range(CHAPTERS):
        # 5 double-group chapters plus 13 single-group chapters = 23 divs.
        count = 2 if chapter < 5 else 1
        body = [f'<title><b>Глава {chapter + 1}</b><br/>Метка {chapter + 1}</title>']
        body.extend(groups[offset:offset + count])
        offset += count
        if 4 <= chapter < 14:
            body.append(f'<p>Краткое пояснение {chapter + 1}</p>')
        all_chapters.append(''.join(body))
    assert offset == len(groups)
    return all_chapters


def generate(folder: Path, mode: str) -> None:
    kind = 'single-p' if mode == 'single-p' else 'single-br'
    content = chapters(kind)
    parts = content if mode == 'split-br' else [''.join(content)]
    paths = [f'chapter{index + 1}.xhtml' for index in range(len(parts))]
    manifest = ''.join(
        f'<item id="chapter{i}" href="{path}" media-type="application/xhtml+xml"/>'
        for i, path in enumerate(paths)
    )
    spine = ''.join(f'<itemref idref="chapter{i}"/>' for i in range(len(parts)))
    opf = (f'<?xml version="1.0"?><package version="2.0" '
           f'xmlns="http://www.idpf.org/2007/opf" unique-identifier="id">'
           f'<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">'
           f'<dc:title>Diagnostic Synthetic EPUB</dc:title><dc:language>ru</dc:language>'
           f'<dc:identifier id="id">urn:uuid:00000000-0000-4000-8000-000000000001</dc:identifier>'
           f'</metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>')
    path = folder / f'{mode}.epub'
    with ZipFile(path, 'w') as epub:
        epub.writestr('mimetype', 'application/epub+zip', compress_type=ZIP_STORED)
        epub.writestr('META-INF/container.xml', CONTAINER, compress_type=ZIP_DEFLATED)
        epub.writestr('OPS/content.opf', opf, compress_type=ZIP_DEFLATED)
        for item_path, part in zip(paths, parts, strict=True):
            epub.writestr(f'OPS/{item_path}', ROOT.format(part), compress_type=ZIP_DEFLATED)
    with ZipFile(path) as epub:
        assert epub.testzip() is None
        for chapter in paths:
            ElementTree.fromstring(epub.read(f'OPS/{chapter}'))
    print(f'SYNTHETIC_EPUB mode={mode} sections={len(parts)} archive_bytes={path.stat().st_size} '
          f'xhtml_bytes={sum(len(ROOT.format(part).encode()) for part in parts)} lines={LINES}', flush=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    for kind in ('single-br', 'single-p', 'split-br'):
        generate(args.out, kind)


if __name__ == '__main__':
    main()
