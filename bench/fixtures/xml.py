from lxml import etree

def parse(raw):
    parser = etree.XMLParser(resolve_entities=True)
    return etree.fromstring(raw, parser)
