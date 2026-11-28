WITH clipped AS (
    SELECT MAX(g.start_utc_ms,:lo) AS lo, MIN(g.end_utc_ms,:hi) AS hi,
           g.session_id,g.seq
    FROM segment g JOIN session s ON s.id=g.session_id
    WHERE s.account=:account AND g.start_utc_ms < :hi AND g.end_utc_ms > :lo
), ordered AS (
    SELECT *, MAX(hi) OVER (ORDER BY lo,hi,session_id,seq ROWS BETWEEN
      UNBOUNDED PRECEDING AND 1 PRECEDING) AS prev_max FROM clipped
), islands AS (
    SELECT *, SUM(CASE WHEN prev_max IS NULL OR lo > prev_max THEN 1 ELSE 0 END)
    OVER (ORDER BY lo,hi,session_id,seq ROWS UNBOUNDED PRECEDING) AS island
    FROM ordered
), spans AS (
    SELECT MIN(lo) AS lo, MAX(hi) AS hi FROM islands GROUP BY island
)
SELECT (SELECT COALESCE(SUM(hi-lo),0) FROM spans),
       (SELECT COALESCE(SUM(hi-lo),0) FROM clipped);
