# Semaphore cost by caller (build A, self instructions per request)

## rp / ct

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 757.0 |
| 2b: server (database and other server locks) | 228.5 |
| channel permits (bounded mpsc) | 144.0 |

## rp / mt

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 757.0 |
| 2b: server (database and other server locks) | 228.6 |
| channel permits (bounded mpsc) | 144.0 |

## wp / ct

| Group | Self instructions per request |
| --- | --- |
| 2b: server (database and other server locks) | 1,112.6 |
| 2a: client (transaction table and other client locks) | 757.0 |
| channel permits (bounded mpsc) | 144.0 |

## wp / mt

| Group | Self instructions per request |
| --- | --- |
| 2b: server (database and other server locks) | 1,112.7 |
| 2a: client (transaction table and other client locks) | 757.0 |
| channel permits (bounded mpsc) | 144.0 |

## rpm / ct

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 757.0 |
| 2b: server (database and other server locks) | 230.2 |
| channel permits (bounded mpsc) | 144.0 |

## rpm / mt

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 757.2 |
| 2b: server (database and other server locks) | 232.0 |
| channel permits (bounded mpsc) | 144.0 |

