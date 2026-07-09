using System.Buffers.Binary;
using System.Text;

namespace Powder;

/// <summary>One of the four physical column types carried by PCB.</summary>
public enum DataType : byte
{
    Int64 = 0,
    Float64 = 1,
    Bool = 2,
    Utf8 = 3,
}

/// <summary>
/// A single decoded PCB column. Values are read straight out of the
/// little-endian payload; nothing is materialized until you ask for it.
/// </summary>
public sealed class Column
{
    private readonly byte[] _data;
    private readonly int _validityOff; // -1 = no validity bitmap
    private readonly int _buf1Off;
    private readonly int _buf2Off;
    private readonly uint[]? _utf8Offsets;

    internal Column(string name, DataType type, int length, byte[] data,
                    int validityOff, int buf1Off, int buf2Off, uint[]? utf8Offsets)
    {
        Name = name;
        Type = type;
        Length = length;
        _data = data;
        _validityOff = validityOff;
        _buf1Off = buf1Off;
        _buf2Off = buf2Off;
        _utf8Offsets = utf8Offsets;
    }

    public string Name { get; }
    public DataType Type { get; }
    public int Length { get; }

    /// <summary>Whether the slot holds a value (vs. SQL NULL).</summary>
    public bool IsValid(int row)
    {
        if (_validityOff < 0)
        {
            return true;
        }
        return (_data[_validityOff + (row >> 3)] & (1 << (row & 7))) != 0;
    }

    public long GetInt64(int row) =>
        BinaryPrimitives.ReadInt64LittleEndian(_data.AsSpan(_buf1Off + row * 8, 8));

    public double GetDouble(int row) =>
        BinaryPrimitives.ReadDoubleLittleEndian(_data.AsSpan(_buf1Off + row * 8, 8));

    public bool GetBoolean(int row) =>
        (_data[_buf1Off + (row >> 3)] & (1 << (row & 7))) != 0;

    public string GetString(int row)
    {
        var offsets = _utf8Offsets!;
        int start = (int)offsets[row];
        int end = (int)offsets[row + 1];
        return Encoding.UTF8.GetString(_data, _buf2Off + start, end - start);
    }

    /// <summary>Boxed value at <paramref name="row"/>, or null for SQL NULL.</summary>
    public object? Get(int row)
    {
        if (row < 0 || row >= Length || !IsValid(row))
        {
            return null;
        }
        return Type switch
        {
            DataType.Int64 => GetInt64(row),
            DataType.Float64 => GetDouble(row),
            DataType.Bool => GetBoolean(row),
            DataType.Utf8 => GetString(row),
            _ => null,
        };
    }
}

/// <summary>A decoded columnar result set.</summary>
public sealed class Batch
{
    private readonly Dictionary<string, Column> _byName;

    internal Batch(int numRows, List<Column> columns)
    {
        NumRows = numRows;
        Columns = columns;
        _byName = new Dictionary<string, Column>(columns.Count, StringComparer.Ordinal);
        foreach (var c in columns)
        {
            _byName[c.Name] = c;
        }
    }

    public int NumRows { get; }
    public IReadOnlyList<Column> Columns { get; }

    public Column this[string name] =>
        _byName.TryGetValue(name, out var c)
            ? c
            : throw new PowderException($"no such column: {name}");

    /// <summary>Row-oriented view (copies) — convenient for small result sets.</summary>
    public List<Dictionary<string, object?>> ToRows()
    {
        var rows = new List<Dictionary<string, object?>>(NumRows);
        for (int r = 0; r < NumRows; r++)
        {
            var row = new Dictionary<string, object?>(Columns.Count, StringComparer.Ordinal);
            foreach (var c in Columns)
            {
                row[c.Name] = c.Get(r);
            }
            rows.Add(row);
        }
        return rows;
    }

    private const uint Magic = 0x31424350; // "PCB1" little-endian
    private const int ColDirLen = 40;

    /// <summary>Decode a PCB buffer (the wire format in docs/FORMAT.md).</summary>
    public static Batch Decode(byte[] data)
    {
        if (data.Length < 24 || BinaryPrimitives.ReadUInt32LittleEndian(data) != Magic)
        {
            throw new PowderException("not a PCB buffer (bad magic)");
        }
        ushort version = BinaryPrimitives.ReadUInt16LittleEndian(data.AsSpan(4, 2));
        if (version != 1)
        {
            throw new PowderException($"unsupported PCB version {version}");
        }
        int numColumns = (int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(8, 4));
        int numRows = (int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(12, 4));
        int dirOff = (int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(16, 4));

        var columns = new List<Column>(numColumns);
        for (int c = 0; c < numColumns; c++)
        {
            int d = dirOff + c * ColDirLen;
            int nameOff = (int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(d, 4));
            int nameLen = (int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(d + 4, 4));
            byte typeCode = data[d + 8];
            if (typeCode > 3)
            {
                throw new PowderException($"unsupported PCB type code {typeCode}");
            }
            bool hasValidity = (data[d + 9] & 1) != 0;
            int validityOff = (int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(d + 12, 4));
            int buf1Off = (int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(d + 20, 4));
            int buf2Off = (int)BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(d + 28, 4));

            string name = Encoding.UTF8.GetString(data, nameOff, nameLen);
            uint[]? utf8Offsets = null;
            if ((DataType)typeCode == DataType.Utf8)
            {
                utf8Offsets = new uint[numRows + 1];
                for (int i = 0; i <= numRows; i++)
                {
                    utf8Offsets[i] =
                        BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(buf1Off + i * 4, 4));
                }
            }
            columns.Add(new Column(name, (DataType)typeCode, numRows, data,
                                   hasValidity ? validityOff : -1, buf1Off, buf2Off, utf8Offsets));
        }
        return new Batch(numRows, columns);
    }
}
