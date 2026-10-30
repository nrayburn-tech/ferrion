package ferrion.oracle;

/** A minimal JSON writer, so the harness has no dependencies. */
final class Json {
  private final StringBuilder out = new StringBuilder();
  private boolean needsComma;

  Json beginObject() {
    separate();
    out.append('{');
    needsComma = false;
    return this;
  }

  Json endObject() {
    out.append('}');
    needsComma = true;
    return this;
  }

  Json beginArray() {
    separate();
    out.append('[');
    needsComma = false;
    return this;
  }

  Json endArray() {
    out.append(']');
    needsComma = true;
    return this;
  }

  Json name(String name) {
    value(name);
    out.append(':');
    needsComma = false;
    return this;
  }

  Json value(long value) {
    separate();
    out.append(value);
    needsComma = true;
    return this;
  }

  Json value(String value) {
    separate();
    out.append('"');
    for (int i = 0; i < value.length(); i++) {
      char c = value.charAt(i);
      switch (c) {
        case '"' -> out.append("\\\"");
        case '\\' -> out.append("\\\\");
        case '\n' -> out.append("\\n");
        case '\r' -> out.append("\\r");
        case '\t' -> out.append("\\t");
        default -> {
          if (c < 0x20) {
            out.append(String.format("\\u%04x", (int) c));
          } else {
            out.append(c);
          }
        }
      }
    }
    out.append('"');
    needsComma = true;
    return this;
  }

  private void separate() {
    if (needsComma) {
      out.append(',');
    }
  }

  @Override
  public String toString() {
    return out.toString();
  }
}
