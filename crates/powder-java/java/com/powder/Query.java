package com.powder;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/** Fluent, injection-safe SELECT builder — mirror of the Rust/TS builders. */
public final class Query {
    private final String table;
    private List<String> cols = new ArrayList<>();
    private final List<String> wheres = new ArrayList<>();
    private final List<Object> params = new ArrayList<>();
    private String orderBy;
    private Long limitN;
    private Long offsetN;

    private Query(String table) {
        this.table = table;
    }

    public static Query table(String name) {
        return new Query(name);
    }

    public Query select(String... columns) {
        this.cols = new ArrayList<>(Arrays.asList(columns));
        return this;
    }

    /** Add a WHERE predicate; one {@code ?} per supplied param. ANDed. */
    public Query filter(String predicate, Object... params) {
        this.wheres.add(predicate);
        this.params.addAll(Arrays.asList(params));
        return this;
    }

    public Query order(String column, String direction) {
        this.orderBy = column + " " + ("DESC".equalsIgnoreCase(direction) ? "DESC" : "ASC");
        return this;
    }

    public Query order(String column) {
        return order(column, "ASC");
    }

    public Query limit(long n) {
        this.limitN = n;
        return this;
    }

    public Query offset(long n) {
        this.offsetN = n;
        return this;
    }

    public String sql() {
        StringBuilder sb = new StringBuilder("SELECT ");
        sb.append(cols.isEmpty() ? "*" : String.join(", ", cols));
        sb.append(" FROM ").append(table);
        if (!wheres.isEmpty()) {
            sb.append(" WHERE ").append(String.join(" AND ", wheres));
        }
        if (orderBy != null) {
            sb.append(" ORDER BY ").append(orderBy);
        }
        if (limitN != null) {
            sb.append(" LIMIT ").append(limitN);
        }
        if (offsetN != null) {
            sb.append(" OFFSET ").append(offsetN);
        }
        return sb.toString();
    }

    public Object[] params() {
        return params.toArray();
    }
}
