# Semaphore cost by caller (build A, self instructions per request)

## rp / ct

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 826.8 |
| 2b: server (database and other server locks) | 298.2 |
| channel permits (bounded mpsc) | 4.5 |

## rp / mt

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 826.8 |
| 2b: server (database and other server locks) | 298.3 |
| channel permits (bounded mpsc) | 4.5 |

## wp / ct

| Group | Self instructions per request |
| --- | --- |
| 2b: server (database and other server locks) | 1,182.3 |
| 2a: client (transaction table and other client locks) | 826.8 |
| channel permits (bounded mpsc) | 4.5 |

## wp / mt

| Group | Self instructions per request |
| --- | --- |
| 2b: server (database and other server locks) | 1,182.5 |
| 2a: client (transaction table and other client locks) | 826.8 |
| channel permits (bounded mpsc) | 4.5 |

## rpm / ct

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 826.8 |
| 2b: server (database and other server locks) | 299.9 |
| channel permits (bounded mpsc) | 4.5 |

## rpm / mt

| Group | Self instructions per request |
| --- | --- |
| 2a: client (transaction table and other client locks) | 826.9 |
| 2b: server (database and other server locks) | 301.7 |
| channel permits (bounded mpsc) | 4.5 |

