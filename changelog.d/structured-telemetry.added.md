Runs and birds now emit structured lifecycle logs to stderr. An optional
OTLP/HTTP build can send those logs, correlated run spans, and workload
request counters and latency histograms to an explicitly selected collector.
The live flock does not require a collector.
