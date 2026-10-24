program LanguageCompare;
{$mode objfpc}
uses SysUtils;
const LIMIT = 10000000;
var
  marked: array of byte;
  p, j, i, count: LongInt;
  hash: LongWord;
begin
  SetLength(marked, LIMIT + 1);
  p := 2;
  while p*p <= LIMIT do begin
    if marked[p] = 0 then begin
      j := p*p;
      while j <= LIMIT do begin marked[j] := 1; Inc(j, p); end;
    end;
    Inc(p);
  end;
  count := 0;
  for i := 2 to LIMIT do if marked[i] = 0 then Inc(count);
  hash := 2166136261;
  for i := 0 to 19999999 do
    hash := (hash xor LongWord((Int64(i)*131 + 17) and 255)) * 16777619;
  WriteLn(count);
  WriteLn(hash);
end.
