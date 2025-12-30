export const rangeToA1 = (range: { startRow: number; startCol: number; endRow: number; endCol: number }) => {
  const startCell = `${columnToLetter(range.startCol)}${range.startRow}`
  const endCell = `${columnToLetter(range.endCol)}${range.endRow}`

  // If it's a single cell, just return the cell reference
  if (range.startRow === range.endRow && range.startCol === range.endCol) {
    return startCell
  }

  // If it's a range, return A1:B2 format
  return `${startCell}:${endCell}`
}

export const columnToLetter = (col: number): string => {
  let temp = col
  let letter = ''
  while (temp > 0) {
    const mod = (temp - 1) % 26
    letter = String.fromCharCode(65 + mod) + letter
    temp = Math.floor((temp - 1) / 26)
  }
  return letter
}

export function formatRangeDisplay(
  a1Notation: string,
  sheetTitle: string,
  currentSheetTitle: string | null
): string {
  // If the range is from a different sheet than the current one, include the sheet name
  if (currentSheetTitle && sheetTitle !== currentSheetTitle) {
    return `${sheetTitle}!${a1Notation}`;
  }
  return a1Notation;
}