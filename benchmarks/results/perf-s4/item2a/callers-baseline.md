# Semaphore cost by caller (build A, self instructions per request)

## rp / ct

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 773.0 |
| 2b: server (database and other server locks) | 228.6 |
| channel permits (bounded mpsc) | 144.0 |

## rp / mt

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 773.2 |
| 2b: server (database and other server locks) | 236.1 |
| channel permits (bounded mpsc) | 144.0 |

## wp / ct

| Group | Self instructions per request |
| --- | --- |
| 2b: server (database and other server locks) | 1,145.9 |
| 2a: client (transaction table and other client locks) | 773.0 |
| channel permits (bounded mpsc) | 144.0 |

## wp / mt

| Group | Self instructions per request |
| --- | --- |
| 2b: server (database and other server locks) | 1,146.7 |
| 2a: client (transaction table and other client locks) | 773.3 |
| channel permits (bounded mpsc) | 144.0 |

## rpm / ct

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 773.0 |
| 2b: server (database and other server locks) | 230.6 |
| channel permits (bounded mpsc) | 144.0 |

## rpm / mt

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 773.2 |
| 2b: server (database and other server locks) | 230.7 |
| channel permits (bounded mpsc) | 144.0 |

