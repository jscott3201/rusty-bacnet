# Semaphore cost by caller (build A, self instructions per request)

## rp / ct

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 757.0 |
| 2b: server (database and other server locks) | 228.7 |
| channel permits (bounded mpsc) | 144.0 |

## rp / mt

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 757.0 |
| 2b: server (database and other server locks) | 228.0 |
| channel permits (bounded mpsc) | 144.0 |

## wp / ct

| Group | Self instructions per request |
| --- | --- |
| 2b: server (database and other server locks) | 1,112.8 |
| 2a: client (transaction table and other client locks) | 757.0 |
| channel permits (bounded mpsc) | 144.0 |

## wp / mt

| Group | Self instructions per request |
| --- | --- |
| 2b: server (database and other server locks) | 1,113.5 |
| 2a: client (transaction table and other client locks) | 757.2 |
| channel permits (bounded mpsc) | 144.0 |

## rpm / ct

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 757.0 |
| 2b: server (database and other server locks) | 230.8 |
| channel permits (bounded mpsc) | 144.0 |

## rpm / mt

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 757.1 |
| 2b: server (database and other server locks) | 231.2 |
| channel permits (bounded mpsc) | 144.0 |

