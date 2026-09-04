SELECT id,
  nid,
  ord,
  cast(mod AS integer),
  did,
  odid
FROM cards
WHERE queue = 0